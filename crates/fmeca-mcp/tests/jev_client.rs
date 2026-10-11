//! The Jev client: configuration, key redaction and HTTP error mapping
//! against a local mock server. No test here reaches the network, except
//! the `#[ignore]`d live one.
//!
//! Run: `cargo test -p fmeca-mcp --features test-fakes --test jev_client`.

#![cfg(feature = "jev")]

use std::collections::BTreeMap;
use std::time::Duration;

use fmeca_mcp::jev::{
    Answer, DEFAULT_JEV_MODEL, DEFAULT_TIMEOUT_SECS, JevConfig, JudgmentModel,
    OPENROUTER_API_KEY_ENV, OpenRouterJudge, Question, Unavailable, UnavailableReason,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-or-v1-test-0123456789abcdef";

fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| (*v).to_string())
    }
}

fn config_with_key() -> JevConfig {
    JevConfig::from_lookup(lookup(&[(OPENROUTER_API_KEY_ENV, KEY)]))
        .expect("valid config")
        .expect("key is set")
}

fn noul_question() -> BTreeMap<String, Question> {
    BTreeMap::from([(
        "q".to_string(),
        Question::Noul {
            instructions: json!("Does the observation apply?"),
            criteria: None,
        },
    )])
}

/// A judge pointed at `server` with a `timeout`.
fn judge_for(server: &MockServer, timeout: Duration) -> OpenRouterJudge {
    let mut config = config_with_key();
    config.endpoint = format!("{}/api/v1/systemone", server.uri());
    config.timeout = timeout;
    OpenRouterJudge::new(&config)
}

/// One `decide` against a server answering every POST with `response`.
async fn decide_against(response: ResponseTemplate, timeout: Duration) -> Result<f64, Unavailable> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/systemone"))
        .respond_with(response)
        .mount(&server)
        .await;
    let judge = judge_for(&server, timeout);
    let decisions = judge
        .decide(json!({"failure_mode": "fm-1"}), noul_question())
        .await?;
    Ok(decisions.cost.unwrap_or(f64::NAN))
}

async fn failure_of(response: ResponseTemplate) -> Unavailable {
    decide_against(response, Duration::from_secs(5))
        .await
        .expect_err("the call fails")
}

#[test]
fn missing_key_is_none() {
    assert!(JevConfig::from_lookup(|_| None).unwrap().is_none());
}

#[test]
fn default_model_is_jev_1_13() {
    assert_eq!(config_with_key().model, DEFAULT_JEV_MODEL);
}

#[test]
fn default_timeout_is_30s() {
    assert_eq!(
        config_with_key().timeout,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS)
    );
}

#[test]
fn timeout_out_of_range_is_bad_config() {
    let result = JevConfig::from_lookup(lookup(&[
        (OPENROUTER_API_KEY_ENV, KEY),
        (fmeca_mcp::jev::JEV_TIMEOUT_ENV, "0"),
    ]));
    assert_eq!(
        result.map(|_| ()).map_err(|e| e.reason),
        Err(UnavailableReason::BadConfig)
    );
}

#[test]
fn debug_redacts_key() {
    assert!(!format!("{:?}", config_with_key()).contains(KEY));
}

#[tokio::test]
async fn success_decodes_answers_and_numeric_cost() {
    let body = json!({
        "model": "typesafe/jev-1.13-20260917",
        "answers": {"q": {"type": "noul", "noul": 0.8}},
        "usage": {"input_tokens": 10, "output_tokens": 2, "cost": 0.0004}
    });
    let cost = decide_against(
        ResponseTemplate::new(200).set_body_json(body),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(cost, Ok(0.0004));
}

#[tokio::test]
async fn http_401_maps_to_http_4xx_without_key() {
    let error =
        failure_of(ResponseTemplate::new(401).set_body_string(format!("bad key {KEY}"))).await;
    assert!(error.reason == UnavailableReason::Http4xx && !error.detail.contains(KEY));
}

#[tokio::test]
async fn http_429_maps_to_http_4xx() {
    let error = failure_of(ResponseTemplate::new(429).set_body_string("slow down")).await;
    assert_eq!(error.reason, UnavailableReason::Http4xx);
}

#[tokio::test]
async fn http_503_maps_to_http_5xx() {
    let error = failure_of(ResponseTemplate::new(503).set_body_string("unavailable")).await;
    assert_eq!(error.reason, UnavailableReason::Http5xx);
}

#[tokio::test]
async fn malformed_body_maps_to_bad_response() {
    let error = failure_of(ResponseTemplate::new(200).set_body_string("{not json")).await;
    assert_eq!(error.reason, UnavailableReason::BadResponse);
}

#[tokio::test]
async fn missing_answer_maps_to_bad_response() {
    let body = json!({"model": "m", "answers": {"other": {"type": "noul", "noul": 0.5}}});
    let error = failure_of(ResponseTemplate::new(200).set_body_json(body)).await;
    assert_eq!(error.reason, UnavailableReason::BadResponse);
}

#[tokio::test]
async fn slow_reply_maps_to_timeout() {
    let body = json!({"model": "m", "answers": {"q": {"type": "noul", "noul": 0.5}}});
    let error = decide_against(
        ResponseTemplate::new(200)
            .set_body_json(body)
            .set_delay(Duration::from_secs(3)),
        Duration::from_secs(1),
    )
    .await
    .expect_err("the call times out");
    assert_eq!(error.reason, UnavailableReason::Timeout);
}

#[tokio::test]
async fn refused_connection_maps_to_network() {
    let mut config = config_with_key();
    // Port 9 (discard) on IPv6 loopback refuses at once. 127.0.0.1:9 does on
    // the CI runners, but under WSL2 mirrored networking a closed IPv4
    // loopback port hangs instead of refusing.
    config.endpoint = "http://[::1]:9/api/v1/systemone".to_string();
    config.timeout = Duration::from_secs(5);
    let error = OpenRouterJudge::new(&config)
        .decide(json!({"failure_mode": "fm-1"}), noul_question())
        .await
        .map(|_| ())
        .map_err(|e| e.reason);
    assert_eq!(error, Err(UnavailableReason::Network));
}

/// Spends about a cent on OpenRouter. Run explicitly with
/// `cargo test -p fmeca-mcp --features test-fakes --test jev_client -- --ignored`.
#[tokio::test]
#[ignore = "live OpenRouter call; needs OPENROUTER_API_KEY and spends money"]
async fn live_openrouter_answers_a_noul() {
    let config = JevConfig::from_env()
        .expect("valid config")
        .expect("OPENROUTER_API_KEY is set");
    let state = json!({
        "failure_mode": {
            "description": "Order write times out when the primary DB fails over",
            "effect": "customer sees 500"
        }
    });
    let questions = BTreeMap::from([(
        "q".to_string(),
        Question::Noul {
            instructions: json!("Is the effect visible to customers?"),
            criteria: None,
        },
    )]);
    let decisions = OpenRouterJudge::new(&config)
        .decide(state, questions)
        .await
        .expect("live call succeeds");
    assert!(matches!(
        decisions.answers.get("q"),
        Some(Answer::Noul { noul }) if (0.0..=1.0).contains(noul)
    ));
}
