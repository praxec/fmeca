//! Live smoke test of Jev on OpenRouter's System One endpoint (spike #22).
//!
//! Sends ONE request with three questions (Noul, Score, Choice) about a
//! fixed FMECA state, through rig-typesafeai's `JevConfig` wire and rig's
//! HTTP driver, and prints the parsed answers, the reported model, `usage`
//! (with `cost`) and the wall latency. The key is read from
//! `OPENROUTER_API_KEY` and is never printed.
//!
//! The request/reply path is ported from cpm-planner `src/llm/jev.rs`: the
//! request is rig-typesafeai's, only the reply type is ours (`Reply`), because
//! rig-typesafeai 0.44 decodes `usage` as rig-core's `Usage`, whose `cost`
//! must be an object with a `total`, while OpenRouter reports `usage.cost` as
//! a bare number. `lenient_usage` reads `usage` tolerantly.
//!
//! Run: `cargo run -p fmeca-mcp --example jev_openrouter_smoke` (spends money).

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use rig_core::completion::request::Cost;
use rig_core::driver::Model;
use rig_core::error::EncodeError;
use rig_core::operation::Whole;
use rig_core::wire::{
    Call, Descriptor, Encoded, Free, Json, Mode, Operation, Wire, WireFrame, document,
};
use rig_typesafeai::types::{Answer, Question, Usage};
use rig_typesafeai::{DynamicQuery, JevConfig, Query};
use serde::Deserialize;
use serde_json::json;

const MODEL: &str = "typesafe/jev-1.13";
const ENDPOINT: &str = "https://openrouter.ai/api/v1/systemone";

/// One Jev evaluation whose reply decodes as [`Reply`].
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

/// rig-typesafeai's [`JevConfig`] wire, answering with [`Reply`].
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

/// A Jev reply. For the spike, fields we do not model are kept in `other`
/// so the response shape can be recorded.
#[derive(Debug, Deserialize)]
struct Reply {
    model: String,
    answers: UniqueAnswers,
    #[serde(default)]
    usage: Option<serde_json::Value>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

/// The `answers` object; an empty or repeated question id is a decode error.
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

/// `usage` as rig-core's [`Usage`], tolerating provider dialects, and whether
/// part of it was not understood and dropped. A bare number `cost`
/// (OpenRouter) is its total; an object `cost` is read as [`Cost`].
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
        Some(Value::Number(total)) => total.as_f64().map(Cost::from_total),
        Some(other) => serde_json::from_value::<Cost>(other).ok(),
    };
    dropped |= cost.is_none() && raw.is_some_and(|u| !u["cost"].is_null());
    usage.retain(|key, value| {
        let bad = USAGE_COUNTERS.contains(&key.as_str()) && !value.is_u64() && !value.is_null();
        dropped |= bad;
        !bad
    });
    match serde_json::from_value::<Usage>(Value::Object(usage)) {
        Ok(usage) => (Some(usage.cost(cost)), dropped),
        Err(_) => (cost.map(|cost| Usage::new().cost(cost)), true),
    }
}

fn state() -> serde_json::Value {
    json!({
        "failure_mode": {
            "id": "fm-db-timeout",
            "component": "orders-api",
            "description": "Order write times out when the primary DB fails over",
            "cause": "no retry with idempotency key during failover",
            "effect": "customer sees 500; order may be double-submitted",
            "severity_observations": ["sev-customer-visible"],
            "probability_observations": ["prob-occurs-monthly"]
        },
        "mitigation": {
            "id": "m1",
            "kind": "detection",
            "description": "alert on p99 write latency > 2s"
        }
    })
}

fn questions() -> BTreeMap<String, Question> {
    BTreeMap::from([
        (
            "q_noul".to_string(),
            Question::Noul {
                instructions: json!(
                    "Given the cause and effect, does the observation 'customer-visible impact' apply?"
                ),
                criteria: None,
            },
        ),
        (
            "q_score".to_string(),
            Question::Score {
                instructions: json!("How fully does mitigation m1 address the cause?"),
                criteria: ["not at all", "slightly", "partly", "mostly", "fully"]
                    .iter()
                    .map(|s| json!(s))
                    .collect(),
            },
        ),
        (
            "q_choice".to_string(),
            Question::Choice {
                instructions: json!("Which class best describes m1?"),
                criteria: BTreeMap::from([
                    (
                        "prevention".to_string(),
                        Some(json!("stops the failure mode from occurring")),
                    ),
                    (
                        "detection".to_string(),
                        Some(json!("reveals the failure mode when it occurs")),
                    ),
                    (
                        "fail_fast".to_string(),
                        Some(json!("bounds the effect by failing quickly and safely")),
                    ),
                ]),
            },
        ),
    ])
}

#[tokio::main]
async fn main() {
    let Ok(key) = std::env::var("OPENROUTER_API_KEY") else {
        eprintln!("OPENROUTER_API_KEY not set");
        std::process::exit(2);
    };
    if key.is_empty() {
        eprintln!("OPENROUTER_API_KEY not set");
        std::process::exit(2);
    }
    let scrub = |text: &str| text.replace(&key, "[redacted]");

    let wire = JevConfig::new(&key).model(MODEL).with_endpoint(ENDPOINT);
    // reqwest is built with `rustls-no-provider`: install ring first.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = rig_reqwest::ReqwestClient::from(
        rig_reqwest::reqwest::Client::builder()
            .build()
            .expect("reqwest client builds"),
    );
    let model = Model::new(JevWire(wire), client);

    let questions = questions();
    let query = DynamicQuery::new(questions.clone()).expect("questions are valid");
    let request_body = json!({ "model": MODEL, "state": state(), "questions": questions });
    println!("endpoint: POST {ENDPOINT}");
    println!("headers: Authorization: Bearer [redacted], Content-Type: application/json");
    println!(
        "request body (as encoded by the wire, modulo key order):\n{}",
        serde_json::to_string_pretty(&request_body).expect("json")
    );

    let request = rig_typesafeai::types::Request {
        state: state(),
        questions: serde_json::value::to_raw_value(&query).expect("questions serialize"),
    };

    let started = Instant::now();
    let result = model.call(request).await;
    let latency = started.elapsed();
    println!("wall latency: {} ms", latency.as_millis());

    let reply = match result {
        Ok(reply) => reply,
        Err(error) => {
            eprintln!("jev call failed: {}", scrub(&format!("{error:?}")));
            std::process::exit(1);
        }
    };

    println!("model: {}", scrub(&reply.model));
    println!(
        "raw usage: {}",
        scrub(&serde_json::to_string(&reply.usage).expect("json"))
    );
    let (usage, dropped) = lenient_usage(reply.usage.as_ref());
    println!("parsed usage: {usage:?} (dropped parts: {dropped})");
    if !reply.other.is_empty() {
        println!(
            "other reply fields: {}",
            scrub(&serde_json::to_string(&reply.other).expect("json"))
        );
    }
    println!(
        "raw answers: {}",
        scrub(&serde_json::to_string_pretty(&reply.answers.0).expect("json"))
    );

    // rig's own answer validation: ids match, kinds and distributions fit.
    let answers = match query.decode(reply.answers.0) {
        Ok(answers) => answers,
        Err(error) => {
            eprintln!("answer validation failed: {}", scrub(&error.to_string()));
            std::process::exit(1);
        }
    };
    for (id, answer) in &answers {
        match answer {
            Answer::Noul { noul } => println!("{id}: noul P(true) = {noul:.3}"),
            Answer::Score {
                score,
                probabilities,
                confidence,
                ..
            } => println!(
                "{id}: score {score:.3} (confidence {confidence:.3}), probabilities {probabilities:?}"
            ),
            Answer::Choice {
                choice,
                probabilities,
                confidence,
            } => println!(
                "{id}: choice {choice} (confidence {confidence:.3}), probabilities {probabilities:?}"
            ),
        }
    }
}
