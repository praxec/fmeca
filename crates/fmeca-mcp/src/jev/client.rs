//! [`JudgmentModel`] over Jev, via `rig-typesafeai`, pointed at OpenRouter.
//!
//! The request is `rig-typesafeai`'s (its [`JevConfig`] wire encodes it and
//! [`DynamicQuery`] validates questions and answers), sent by rig's HTTP
//! driver. Only the reply type is ours (`Reply`): rig-typesafeai 0.44
//! decodes `usage` as rig-core's `Usage`, whose `cost` must be an object
//! with a `total`, while OpenRouter reports `usage.cost` as a bare number,
//! so every OpenRouter reply would fail to decode. `Reply` reads `usage`
//! leniently: `cost` may be a non-negative number or an object, unknown
//! fields are ignored, and malformed accounting is dropped rather than
//! failing the judgment.
//!
//! Every [`Unavailable::detail`] is built through [`Unavailable::scrubbed`]:
//! scrubbed of the key, capped at 300 chars, and limited to an HTTP status
//! or a short rig-authored reason. Upstream bodies go to the `debug` log
//! only (scrubbed, cut, and Debug-escaped so a body cannot forge log lines).
//!
//! TLS is rustls with the `ring` crypto provider, installed process-wide
//! (if no provider is installed yet) before each judge builds its client.
//!
//! Ported from cpm-planner `src/llm/jev.rs` and adapted.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use rig_core::completion::request::Cost;
use rig_core::driver::Model;
use rig_core::error::{EncodeError, ProviderError};
use rig_core::operation::Whole;
use rig_core::wire::{
    Call, Descriptor, Encoded, Free, Json, Mode, Operation, Wire, WireFrame, document,
};
use rig_typesafeai::{DynamicQuery, JevConfig, Query};
use serde::Deserialize;

use super::{
    Answer, ApiKey, Decisions, JudgmentModel, MAX_REPORTED_MODEL_CHARS, Question, Unavailable,
    UnavailableReason, Usage,
};

/// Longest slice of a provider error body written to the debug log.
const BODY_LOG_CHARS: usize = 200;

/// One Jev evaluation whose reply decodes as [`Reply`]; otherwise
/// rig-typesafeai's `Evaluation` (same request, one whole JSON reply).
struct Evaluation;

impl Operation for Evaluation {
    type Request = rig_typesafeai::types::Request;
    type Event = std::convert::Infallible;
    type End = Reply;
    type Response = Reply;
    type Fold = Whole<Self>;
    type Emit = Free;

    fn fold(_request: &Self::Request, _call: &mut Call<'_>) -> Whole<Self> {
        Whole::<Self>::new()
    }
}

/// rig-typesafeai's [`JevConfig`] wire (endpoint, model, bearer key, request
/// body), answering with [`Reply`].
#[derive(Clone)]
struct JevWire(JevConfig);

impl Wire for JevWire {
    type Op = Evaluation;
    type Payload = Encoded;
    type Frame = WireFrame;
    type Decoder<'id> = Json;
    type Reassembler = document::Unreassembled;

    fn describe(&self) -> Descriptor<'_> {
        self.0.describe()
    }

    fn encode(
        &self,
        request: rig_typesafeai::types::Request,
        mode: Mode,
    ) -> Result<Encoded, EncodeError> {
        self.0.encode(request, mode)
    }

    fn decoder<'id>(&self) -> Self::Decoder<'id> {
        Json
    }
}

/// A Jev reply. Fields other than these are ignored.
#[derive(Debug, Deserialize)]
struct Reply {
    model: String,
    answers: UniqueAnswers,
    /// Read by [`lenient_usage`] after decoding, so bad accounting never
    /// fails the reply.
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

/// The `answers` object; like rig-typesafeai, an empty or repeated question
/// id is a decode error rather than a silently merged map.
#[derive(Debug)]
struct UniqueAnswers(BTreeMap<String, Answer>);

impl<'de> Deserialize<'de> for UniqueAnswers {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = UniqueAnswers;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object keyed by unique question IDs")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<UniqueAnswers, M::Error> {
                let mut answers = BTreeMap::new();
                while let Some((id, answer)) = map.next_entry::<String, Answer>()? {
                    if id.is_empty() || answers.insert(id, answer).is_some() {
                        return Err(serde::de::Error::custom(
                            "question IDs must be nonempty and unique",
                        ));
                    }
                }
                Ok(UniqueAnswers(answers))
            }
        }
        deserializer.deserialize_map(Visit)
    }
}

/// The `usage` counters rig-core's [`Usage`] reads, each a `u64`.
const USAGE_COUNTERS: [&str; 7] = [
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "cached_input_tokens",
    "cache_creation_input_tokens",
    "tool_use_prompt_tokens",
    "reasoning_tokens",
];

/// A usable cost: finite and not negative.
fn plausible(cost: Cost) -> Option<Cost> {
    (cost.total.is_finite() && cost.total >= 0.0).then_some(cost)
}

/// `usage` as rig-core's [`Usage`], tolerating provider dialects, and
/// whether part of it was not understood and dropped. A bare number `cost`
/// (OpenRouter) is its total and an object `cost` is read as [`Cost`]; a
/// negative or malformed cost is dropped. Each part is kept or dropped on
/// its own: a bad `cost` or a counter that is not a `u64` is dropped alone,
/// unknown fields are ignored, and a `usage` that is not an object is
/// `None`. Token accounting never fails a judgment.
fn lenient_usage(raw: Option<&serde_json::Value>) -> (Option<Usage>, bool) {
    use serde_json::Value;
    let mut usage = match raw {
        None | Some(Value::Null) => return (None, false),
        Some(Value::Object(usage)) => usage.clone(),
        Some(_) => return (None, true),
    };
    let mut dropped = false;
    let cost = match usage.remove("cost") {
        None | Some(Value::Null) => None,
        Some(Value::Number(total)) => total.as_f64().map(Cost::from_total).and_then(plausible),
        Some(other) => serde_json::from_value::<Cost>(other)
            .ok()
            .and_then(plausible),
    };
    dropped |= cost.is_none() && raw.is_some_and(|u| !u["cost"].is_null());
    usage.retain(|key, value| {
        let bad = USAGE_COUNTERS.contains(&key.as_str()) && !value.is_u64() && !value.is_null();
        dropped |= bad;
        !bad
    });
    match serde_json::from_value::<Usage>(Value::Object(usage)) {
        Ok(usage) => (Some(usage.cost(cost)), dropped),
        // Unreachable after the filter above; kept total for safety.
        Err(_) => (cost.map(|cost| Usage::new().cost(cost)), true),
    }
}

/// Why `state` breaks rig-typesafeai's state rule (a string, object or
/// array), if it does.
fn state_problem(state: &serde_json::Value) -> Option<&'static str> {
    match state {
        serde_json::Value::String(_)
        | serde_json::Value::Object(_)
        | serde_json::Value::Array(_) => None,
        serde_json::Value::Null => Some("state cannot be null"),
        _ => Some("state must be a string, object or array"),
    }
}

/// A reqwest client for one judge, over rustls with the `ring` provider.
///
/// reqwest is built with `rustls-no-provider`, so a process-wide rustls
/// provider must be installed before a client is built; `ring` is installed
/// here unless one already is (a second install is a harmless `Err`).
///
/// Each judge owns its client: rig's shared client pools keep-alive
/// connections across every caller, while each connection's driver task runs
/// on the tokio runtime that opened it, so a judge on another runtime could
/// check out a connection whose runtime is idle or gone. If reqwest cannot
/// build a client, the shared one is used: it reports that build error on
/// every send.
fn own_http_client() -> rig_reqwest::ReqwestClient {
    let _ = rustls::crypto::ring::default_provider().install_default();
    match reqwest::Client::builder().build() {
        Ok(client) => rig_reqwest::ReqwestClient::from(client),
        Err(_) => rig_reqwest::ReqwestClient::default(),
    }
}

/// Jev on OpenRouter's System One endpoint. Every call is bounded by the
/// configured timeout. Each judge owns its HTTP client and connection pool;
/// pooled connections are driven by the runtime that opened them, so use a
/// judge from one tokio runtime (clones share the pool).
#[derive(Clone)]
pub struct OpenRouterJudge {
    model: Model<JevWire, rig_reqwest::ReqwestClient>,
    model_id: String,
    endpoint: String,
    timeout: Duration,
    key: ApiKey,
}

impl OpenRouterJudge {
    /// Build the client from `config`. No network traffic happens here.
    pub fn new(config: &super::JevConfig) -> OpenRouterJudge {
        let wire = JevConfig::new(config.key.expose())
            .model(config.model.as_str())
            .with_endpoint(config.endpoint.as_str());
        Self {
            model: Model::new(JevWire(wire), own_http_client()),
            model_id: config.model.clone(),
            endpoint: config.endpoint.clone(),
            timeout: config.timeout,
            key: config.key.clone(),
        }
    }

    /// The configured Jev model id.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// The configured endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// A tool-facing error: endpoint query/fragment removed, scrubbed of the
    /// key, then capped at 300 chars.
    fn error(&self, reason: UnavailableReason, detail: impl AsRef<str>) -> Unavailable {
        Unavailable::scrubbed(reason, self.redact_endpoint(detail.as_ref()), &self.key)
    }

    /// `text` with the configured endpoint's query string and fragment
    /// replaced by [`REDACTED`](super::REDACTED): transport errors quote the
    /// request URL, and a query may carry a token.
    fn redact_endpoint(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some((_, rest)) = self.endpoint.split_once(['?', '#']) {
            for part in rest.split(['?', '#']).filter(|p| !p.is_empty()) {
                out = out.replace(part, super::REDACTED);
            }
        }
        out
    }

    /// [`ApiKey::excerpt`] of `text` with the endpoint redacted, for logs.
    fn log_excerpt(&self, text: &str) -> String {
        self.key
            .excerpt(&self.redact_endpoint(text), BODY_LOG_CHARS)
    }

    /// The upstream body goes to the debug log only (scrubbed, then cut);
    /// the tool-facing detail carries the status alone. 400..=499 is
    /// [`UnavailableReason::Http4xx`], 500..=599 is
    /// [`UnavailableReason::Http5xx`], anything else (a 2xx error envelope,
    /// no status) is [`UnavailableReason::BadResponse`].
    fn reply_error(&self, reply: &rig_core::ProviderResponseError) -> Unavailable {
        let status = reply.status.map(|s| s.as_u16());
        tracing::debug!(
            ?status,
            body = ?self.log_excerpt(reply.body.trim()),
            "jev upstream error reply"
        );
        let reason = match status {
            Some(400..=499) => UnavailableReason::Http4xx,
            Some(500..=599) => UnavailableReason::Http5xx,
            _ => UnavailableReason::BadResponse,
        };
        match status {
            Some(code) => self.error(reason, format!("status {code}")),
            None => self.error(reason, "provider error reply without an HTTP status"),
        }
    }

    /// A failure whose text may be provider-authored: logged at debug,
    /// replaced by `summary` in the tool-facing detail.
    fn opaque_error(
        &self,
        reason: UnavailableReason,
        summary: &str,
        error: &ProviderError,
    ) -> Unavailable {
        tracing::debug!(
            detail = ?self.log_excerpt(&error.to_string()),
            "jev provider failure"
        );
        self.error(reason, summary)
    }

    fn classify(&self, error: &ProviderError) -> Unavailable {
        use UnavailableReason as R;
        match error {
            ProviderError::InvalidAuthentication(reply)
            | ProviderError::ProviderResponse(reply) => self.reply_error(reply),
            ProviderError::CacheExpired { response, .. } => self.reply_error(response),
            // rig-authored, short reasons (serde position, id mismatch).
            ProviderError::Json(_)
            | ProviderError::Response(_)
            | ProviderError::Truncated
            | ProviderError::MismatchedDimensions { .. } => {
                self.error(R::BadResponse, error.to_string())
            }
            // Connect, DNS, TLS, reset: no reply was made.
            ProviderError::Http(_) => self.error(R::Network, error.to_string()),
            // Rejected locally before sending: a bad endpoint, questions or
            // state.
            ProviderError::Url(_)
            | ProviderError::Request(_)
            | ProviderError::UnsupportedOption(_) => self.error(R::BadConfig, error.to_string()),
            ProviderError::Provider(_) | ProviderError::Relayed(_) => {
                self.opaque_error(R::BadResponse, "provider reported a failure", error)
            }
            // `ProviderError` is `#[non_exhaustive]`.
            _ => self.opaque_error(R::BadResponse, "unclassified provider failure", error),
        }
    }
}

impl fmt::Debug for OpenRouterJudge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenRouterJudge")
            .field("model", &self.model_id)
            .field("endpoint", &self.endpoint)
            .field("timeout", &self.timeout)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl JudgmentModel for OpenRouterJudge {
    async fn decide(
        &self,
        state: serde_json::Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<Decisions, Unavailable> {
        let query = DynamicQuery::new(questions).map_err(|e| self.classify(&e))?;
        if let Some(problem) = state_problem(&state) {
            return Err(self.error(UnavailableReason::BadConfig, problem));
        }
        let request = rig_typesafeai::types::Request {
            state,
            questions: serde_json::value::to_raw_value(&query)
                .map_err(|e| self.classify(&e.into()))?,
        };
        let reply = tokio::time::timeout(self.timeout, self.model.call(request))
            .await
            .map_err(|_| {
                self.error(
                    UnavailableReason::Timeout,
                    format!("no reply within {} ms", self.timeout.as_millis()),
                )
            })?
            .map_err(|e| self.classify(&e))?;
        let (usage, dropped) = lenient_usage(reply.usage.as_ref());
        if dropped && let Some(raw) = &reply.usage {
            tracing::debug!(
                usage = ?self.log_excerpt(&raw.to_string()),
                "jev usage not understood; dropped"
            );
        }
        // rig's own answer validation: ids match, kinds and distributions fit.
        let answers = query
            .decode(reply.answers.0)
            .map_err(|e| self.classify(&e))?;
        let cost = usage
            .as_ref()
            .and_then(|usage| usage.cost.as_ref())
            .map(|cost| cost.total);
        Ok(Decisions {
            answers,
            // Provider-authored: scrubbed of the key, then cut.
            model: self.key.excerpt(&reply.model, MAX_REPORTED_MODEL_CHARS),
            usage,
            cost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cost_of(raw: serde_json::Value) -> Option<f64> {
        lenient_usage(Some(&raw))
            .0
            .and_then(|usage| usage.cost)
            .map(|cost| cost.total)
    }

    #[test]
    fn numeric_cost_is_the_total() {
        assert_eq!(cost_of(json!({"cost": 0.000024864})), Some(0.000024864));
    }

    #[test]
    fn object_cost_total_is_read() {
        assert_eq!(cost_of(json!({"cost": {"total": 0.5}})), Some(0.5));
    }

    #[test]
    fn negative_cost_is_dropped() {
        assert_eq!(cost_of(json!({"cost": -0.01})), None);
    }

    #[test]
    fn negative_cost_counts_as_dropped_accounting() {
        assert!(lenient_usage(Some(&json!({"cost": -0.01}))).1);
    }

    #[test]
    fn negative_object_cost_is_dropped() {
        assert_eq!(cost_of(json!({"cost": {"total": -1.0}})), None);
    }

    #[test]
    fn malformed_cost_counts_as_dropped_accounting() {
        assert!(lenient_usage(Some(&json!({"cost": "cheap"}))).1);
    }

    #[test]
    fn bad_counter_keeps_the_cost() {
        assert_eq!(
            cost_of(json!({"input_tokens": -3, "cost": 0.25})),
            Some(0.25)
        );
    }

    #[test]
    fn non_object_usage_is_none() {
        assert!(lenient_usage(Some(&json!(7))).0.is_none());
    }

    #[test]
    fn reply_ignores_unknown_fields() {
        let reply: Result<Reply, _> = serde_json::from_value(json!({
            "id": "gen-1", "provider": "TypeSafe", "model": "m",
            "answers": {"q": {"type": "noul", "noul": 0.5}}
        }));
        assert!(reply.is_ok());
    }

    #[test]
    fn duplicate_answer_ids_fail_to_decode() {
        let reply: Result<Reply, _> = serde_json::from_str(
            r#"{"model":"m","answers":{"q":{"type":"noul","noul":0.5},"q":{"type":"noul","noul":0.5}}}"#,
        );
        assert!(reply.is_err());
    }

    #[test]
    fn judge_debug_redacts_key() {
        let config = super::super::JevConfig::new(ApiKey::new("sk-or-v1-secretsecret"));
        assert!(!format!("{:?}", OpenRouterJudge::new(&config)).contains("secretsecret"));
    }
}
