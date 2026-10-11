//! Jev on OpenRouter: configuration, the redacted key, the advisory
//! [`JudgmentModel`] seam and its failure classes.
//!
//! [`JevConfig::from_env`] reads the OpenRouter key and the Jev settings.
//! `Ok(None)` means "no key configured"; callers report
//! [`UnavailableReason::NoKey`] rather than failing. [`JudgmentModel`] is the
//! one async seam the advisory `vet` tool depends on, so it is testable with
//! `fake::FakeJudge` (behind the `test-fakes` feature). [`client::OpenRouterJudge`] implements it with
//! `rig-typesafeai` pointed at OpenRouter's System One endpoint.
//!
//! # Key hygiene
//!
//! The key lives in an [`ApiKey`] whose `Debug` prints `[redacted]` and
//! which has neither `Display` nor `Serialize`. An [`Unavailable`] is only
//! built through [`Unavailable::scrubbed`] (or with a fixed, key-free
//! detail), which passes the detail through [`ApiKey::excerpt`], so no
//! error, log or tool output can carry the key.
//!
//! Ported from cpm-planner `src/llm/mod.rs` and adapted.

pub mod client;
#[cfg(any(test, feature = "test-fakes"))]
pub mod fake;

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

pub use client::OpenRouterJudge;
pub use rig_typesafeai::types::{Answer, Question, Usage};

/// Environment variable holding the OpenRouter key.
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// Environment variable overriding the Jev model id.
pub const JEV_MODEL_ENV: &str = "FMECA_JEV_MODEL";
/// Environment variable bounding every Jev call, in whole seconds.
pub const JEV_TIMEOUT_ENV: &str = "FMECA_JEV_TIMEOUT_SECS";

/// Default Jev model on OpenRouter.
pub const DEFAULT_JEV_MODEL: &str = "typesafe/jev-1.13";
/// Default Jev endpoint: OpenRouter's System One route.
pub const DEFAULT_JEV_ENDPOINT: &str = "https://openrouter.ai/api/v1/systemone";
/// Default per-call timeout, in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Inclusive bounds for [`JEV_TIMEOUT_ENV`], in seconds.
pub const TIMEOUT_SECS_RANGE: std::ops::RangeInclusive<u64> = 1..=300;
/// Longest accepted [`JEV_MODEL_ENV`] value, in chars.
pub const MAX_MODEL_ID_CHARS: usize = 128;
/// Longest provider-reported model id kept in [`Decisions::model`], in chars.
pub const MAX_REPORTED_MODEL_CHARS: usize = 64;

/// Redaction marker used wherever the key would otherwise appear.
pub const REDACTED: &str = "[redacted]";
/// Upper bound on [`Unavailable::detail`], in chars (it becomes tool output).
pub const MAX_DETAIL_CHARS: usize = 300;
/// Shortest key prefix treated as a leak when it ends a truncated excerpt.
const MIN_KEY_FRAGMENT_CHARS: usize = 8;

/// The OpenRouter key. `Debug` is redacted; there is deliberately no
/// `Display` or `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a raw key.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The raw key. Every call site is a place the key can leak: only the
    /// HTTP client should call this.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// `text` with every occurrence of the key replaced by [`REDACTED`].
    pub fn scrub(&self, text: &str) -> String {
        if self.0.is_empty() {
            text.to_string()
        } else {
            text.replace(&self.0, REDACTED)
        }
    }

    /// At most `max_chars` of `text`, safe to show: the WHOLE text is
    /// scrubbed first, then truncated, and a trailing key prefix of
    /// `MIN_KEY_FRAGMENT_CHARS` (8)+ chars (a key cut by the boundary, or a
    /// partial key in the input) is dropped.
    pub fn excerpt(&self, text: &str, max_chars: usize) -> String {
        let scrubbed = self.scrub(text);
        let mut out: Vec<char> = scrubbed.chars().take(max_chars).collect();
        let key: Vec<char> = self.0.chars().collect();
        let longest = key.len().min(out.len());
        if let Some(len) = (MIN_KEY_FRAGMENT_CHARS..=longest)
            .rev()
            .find(|&len| out[out.len() - len..] == key[..len])
        {
            out.truncate(out.len() - len);
        }
        out.into_iter().collect()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// Jev settings. Only built when a key is configured.
///
/// The fields are public so callers (and tests) can point a judge at another
/// endpoint or bound it differently; [`JevConfig::from_lookup`] validates
/// what comes from the environment.
#[derive(Clone, Debug)]
pub struct JevConfig {
    /// The OpenRouter key (redacted `Debug`).
    pub key: ApiKey,
    /// The Jev model id sent with every request.
    pub model: String,
    /// The complete Jev endpoint URL, route included.
    pub endpoint: String,
    /// The bound applied to every call.
    pub timeout: Duration,
}

impl JevConfig {
    /// Settings for `key` with every other field at its default.
    pub fn new(key: ApiKey) -> Self {
        Self {
            key,
            model: DEFAULT_JEV_MODEL.to_string(),
            endpoint: DEFAULT_JEV_ENDPOINT.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Read the process environment. `Ok(None)` when
    /// [`OPENROUTER_API_KEY_ENV`] is unset or empty.
    pub fn from_env() -> Result<Option<JevConfig>, Unavailable> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] over a supplied lookup (tests use a map, not the
    /// process env).
    ///
    /// A blank value counts as unset. [`JEV_TIMEOUT_ENV`] must be an integer
    /// in [`TIMEOUT_SECS_RANGE`] and [`JEV_MODEL_ENV`] must be 1..=128 chars
    /// of `[A-Za-z0-9._:/-]` without the key; otherwise the result is
    /// [`UnavailableReason::BadConfig`], whose detail names the variable but
    /// never echoes its value. The settings are validated even when no key
    /// is set, so a bad configuration is never reported as `no_key` later.
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<JevConfig>, Unavailable> {
        let non_blank = |name: &str| get(name).filter(|v| !v.trim().is_empty());
        let key = non_blank(OPENROUTER_API_KEY_ENV).map(|k| ApiKey::new(k.trim()));

        let timeout = match non_blank(JEV_TIMEOUT_ENV) {
            Some(raw) => parse_timeout(&raw)?,
            None => Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        };
        let model = match non_blank(JEV_MODEL_ENV) {
            Some(raw) => Some(parse_model(raw.trim(), key.as_ref())?),
            None => None,
        };
        let Some(key) = key else {
            return Ok(None);
        };
        let mut config = Self::new(key);
        config.timeout = timeout;
        if let Some(model) = model {
            config.model = model;
        }
        Ok(Some(config))
    }
}

fn parse_timeout(raw: &str) -> Result<Duration, Unavailable> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|secs| TIMEOUT_SECS_RANGE.contains(secs))
        .map(Duration::from_secs)
        .ok_or_else(|| {
            Unavailable::fixed(
                UnavailableReason::BadConfig,
                format!("{JEV_TIMEOUT_ENV} must be an integer number of seconds in 1..=300"),
            )
        })
}

/// A model id: 1..=[`MAX_MODEL_ID_CHARS`] chars of `[A-Za-z0-9._:/-]`, not
/// containing `key`. The value is never echoed.
fn parse_model(raw: &str, key: Option<&ApiKey>) -> Result<String, Unavailable> {
    let invalid = |rule: &str| {
        Err(Unavailable::fixed(
            UnavailableReason::BadConfig,
            format!("{JEV_MODEL_ENV} {rule}"),
        ))
    };
    if raw.chars().count() > MAX_MODEL_ID_CHARS {
        return invalid("must be at most 128 characters");
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
    {
        return invalid("may only contain A-Z a-z 0-9 . _ : / -");
    }
    if key.is_some_and(|key| !key.expose().is_empty() && raw.contains(key.expose())) {
        return invalid("must not contain the OpenRouter key");
    }
    Ok(raw.to_string())
}

/// Why Jev could not judge. Serialized as snake_case into tool output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// No OpenRouter key is configured.
    NoKey,
    /// No reply within the configured timeout.
    Timeout,
    /// The provider answered 400..=499 (bad key, rate limit, bad request).
    #[serde(rename = "http_4xx")]
    Http4xx,
    /// The provider answered 500..=599.
    #[serde(rename = "http_5xx")]
    Http5xx,
    /// No reply: connection refused or reset, DNS, TLS.
    Network,
    /// The reply arrived but does not decode or does not answer every
    /// question asked.
    BadResponse,
    /// The configuration ([`JEV_TIMEOUT_ENV`], [`JEV_MODEL_ENV`]) or the
    /// request was rejected locally before sending.
    BadConfig,
}

/// A classed judgment failure. `detail` never contains the key and is at
/// most [`MAX_DETAIL_CHARS`] chars: build one with [`Unavailable::scrubbed`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Unavailable {
    /// The failure class.
    pub reason: UnavailableReason,
    /// A short, safe, human-readable detail (an HTTP status, a reason).
    pub detail: String,
}

impl Unavailable {
    /// An error whose detail is [`ApiKey::excerpt`]ed: scrubbed of `key`,
    /// then cut to [`MAX_DETAIL_CHARS`].
    pub fn scrubbed(reason: UnavailableReason, detail: impl AsRef<str>, key: &ApiKey) -> Self {
        Self {
            reason,
            detail: key.excerpt(detail.as_ref(), MAX_DETAIL_CHARS),
        }
    }

    /// An error with a detail written here, not derived from any input that
    /// could carry the key; still cut to [`MAX_DETAIL_CHARS`].
    pub(crate) fn fixed(reason: UnavailableReason, detail: impl AsRef<str>) -> Self {
        Self {
            reason,
            detail: detail.as_ref().chars().take(MAX_DETAIL_CHARS).collect(),
        }
    }

    /// The error for "no key configured".
    pub fn no_key() -> Self {
        Self::fixed(
            UnavailableReason::NoKey,
            format!("{OPENROUTER_API_KEY_ENV} is not set"),
        )
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = serde_json::to_value(self.reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        write!(f, "jev unavailable ({reason}): {}", self.detail)
    }
}

impl std::error::Error for Unavailable {}

/// Jev's answers to one batched evaluation.
#[derive(Debug, Clone)]
pub struct Decisions {
    /// One answer per question id, keyed exactly as asked.
    pub answers: BTreeMap<String, Answer>,
    /// The model id the provider reports having used (often dated, e.g.
    /// `typesafe/jev-1.13-20260917`), scrubbed and cut to
    /// [`MAX_REPORTED_MODEL_CHARS`].
    pub model: String,
    /// Token accounting when the provider reports it.
    pub usage: Option<Usage>,
    /// The call's cost in USD when reported: OpenRouter's bare-number
    /// `usage.cost`, or an object cost's `total`. Never negative.
    pub cost: Option<f64>,
}

/// A calibrated-judgment model: one batched call answering runtime-defined
/// questions about one shared state.
// async-trait marks the boxed-future method `#[must_use]`, which clippy
// 1.99's `double_must_use` flags inside the macro expansion.
#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait JudgmentModel: Send + Sync {
    /// Ask every question in `questions` about `state` in a single call.
    async fn decide(
        &self,
        state: serde_json::Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<Decisions, Unavailable>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-or-v1-abcdefghijklmnop";

    fn key() -> ApiKey {
        ApiKey::new(KEY)
    }

    #[test]
    fn scrub_replaces_every_key_occurrence() {
        assert_eq!(
            key().scrub(&format!("a {KEY} b {KEY}")),
            "a [redacted] b [redacted]"
        );
    }

    #[test]
    fn excerpt_drops_a_key_prefix_cut_at_the_boundary() {
        assert_eq!(
            key().excerpt(&format!("err: {}", &KEY[..12]), 5 + 10),
            "err: "
        );
    }

    #[test]
    fn excerpt_caps_length() {
        assert_eq!(
            key()
                .excerpt(&"x".repeat(1000), MAX_DETAIL_CHARS)
                .chars()
                .count(),
            300
        );
    }

    #[test]
    fn scrubbed_unavailable_detail_omits_key() {
        let error = Unavailable::scrubbed(UnavailableReason::Network, format!("to {KEY}"), &key());
        assert!(!error.detail.contains(KEY));
    }

    #[test]
    fn key_debug_is_redacted() {
        assert_eq!(format!("{:?}", key()), REDACTED);
    }

    #[test]
    fn model_with_bad_chars_is_bad_config() {
        let result = JevConfig::from_lookup(|name| match name {
            OPENROUTER_API_KEY_ENV => Some(KEY.to_string()),
            JEV_MODEL_ENV => Some("typesafe/jev 1.13".to_string()),
            _ => None,
        });
        assert_eq!(
            result.map(|_| ()).map_err(|e| e.reason),
            Err(UnavailableReason::BadConfig)
        );
    }

    #[test]
    fn model_containing_key_is_bad_config_without_echo() {
        let result = JevConfig::from_lookup(|name| match name {
            OPENROUTER_API_KEY_ENV => Some(KEY.to_string()),
            JEV_MODEL_ENV => Some(format!("x{KEY}")),
            _ => None,
        });
        assert!(result.is_err_and(|e| !e.detail.contains(KEY)));
    }

    #[test]
    fn bad_timeout_without_key_is_bad_config() {
        let result =
            JevConfig::from_lookup(|name| (name == JEV_TIMEOUT_ENV).then(|| "301".to_string()));
        assert_eq!(
            result.map(|_| ()).map_err(|e| e.reason),
            Err(UnavailableReason::BadConfig)
        );
    }

    #[test]
    fn reason_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(UnavailableReason::Http4xx).ok(),
            Some(serde_json::json!("http_4xx"))
        );
    }

    #[test]
    fn http_5xx_reason_serializes_with_underscore() {
        assert_eq!(
            serde_json::to_value(UnavailableReason::Http5xx).ok(),
            Some(serde_json::json!("http_5xx"))
        );
    }
}
