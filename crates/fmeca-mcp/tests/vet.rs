//! The advisory `vet` tool: question planning, the cap, deterministic
//! hashes, interpretation and the guarantee that vetting never changes the
//! deterministic outputs. Every test but the `#[ignore]`d live one uses
//! `FakeJudge`, so none reaches the network.
//!
//! Run: `cargo test -p fmeca-mcp --features test-fakes --test vet`.

#![cfg(feature = "test-fakes")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use fmeca::{Engine, FilesystemStore, MatrixStrategy};
use fmeca_mcp::jev::fake::FakeJudge;
use fmeca_mcp::jev::{
    Answer, Decisions, JevConfig, JudgmentModel, OpenRouterJudge, Question, Unavailable,
    UnavailableReason,
};
use fmeca_mcp::vet::{CHECKS, plan_questions, subject_from_analysis};
use fmeca_mcp::{FmecaServer, TOOL_NAMES, TOOL_VET};
use rmcp::model::{CallToolRequestParams, ErrorCode};
use serde_json::{Value, json};

// --- fixtures ----------------------------------------------------------------

fn server() -> (FmecaServer, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(dir.path()).unwrap();
    let engine = Engine::new(Arc::new(store));
    (FmecaServer::new(engine), dir)
}

fn judged(judge: Arc<dyn JudgmentModel>) -> (FmecaServer, tempfile::TempDir) {
    let (srv, dir) = server();
    (srv.with_judge(Some(judge)), dir)
}

fn call(srv: &FmecaServer, name: &str, args: Value) -> Value {
    let Value::Object(map) = args else {
        panic!("call expects a JSON object")
    };
    srv.dispatch_call(CallToolRequestParams::new(name.to_string()).with_arguments(map))
        .unwrap()
}

/// Answers every question the same way: Nouls with `noul`, Scores with
/// `score` (0..=4), Choices picking `choice` with probability 0.8.
fn answer(question: &Question, noul: f64, score: f64, choice: &str) -> Answer {
    match question {
        Question::Noul { .. } => Answer::Noul { noul },
        Question::Score { .. } => Answer::Score {
            score,
            probabilities: BTreeMap::from([(format!("{}", score.round() as u8), 1.0)]),
            legend: BTreeMap::new(),
            confidence: 1.0,
        },
        Question::Choice { criteria, .. } => Answer::Choice {
            choice: choice.to_string(),
            probabilities: criteria
                .keys()
                .map(|k| (k.clone(), if k == choice { 0.8 } else { 0.1 }))
                .collect(),
            confidence: 0.8,
        },
    }
}

fn fake(noul: f64, score: f64, choice: &'static str) -> Arc<FakeJudge> {
    Arc::new(FakeJudge::answering(move |_, q| {
        answer(q, noul, score, choice)
    }))
}

/// One analysis failure mode on the 3×3 catalog with `mitigations`
/// detection mitigations. Claims medium severity and medium probability.
fn analysis_fm(id: &str, mitigations: usize) -> Value {
    json!({
        "id": id,
        "component": { "id": "c:orders" },
        "description": "order write times out during DB failover",
        "cause": "no retry with idempotency key",
        "effect": "customer sees an error",
        "domain": "runtime",
        "severity_observations": ["user_facing_degradation"],
        "probability_observations": ["occasional"],
        "mitigations": (0..mitigations).map(|k| json!({
            "id": format!("{id}-m{k}"),
            "kind": "detection",
            "description": "alert on p99 write latency"
        })).collect::<Vec<_>>()
    })
}

/// `vet` arguments vetting a one-shot analysis of `fms` failure modes.
fn analysis(fms: usize, mitigations: usize) -> Value {
    json!({
        "analysis": {
            "failure_modes": (0..fms)
                .map(|i| analysis_fm(&format!("fm-{i:02}"), mitigations))
                .collect::<Vec<_>>()
        }
    })
}

fn finding(result: &Value, target: &str) -> Value {
    result["findings"]
        .as_array()
        .and_then(|f| f.iter().find(|f| f["target"] == target))
        .cloned()
        .unwrap_or(Value::Null)
}

const SESSION: &str = "vet-session";

/// A session with two failure modes and one mitigation.
fn seeded_session(srv: &FmecaServer) {
    call(srv, "session.open", json!({ "session_id": SESSION }));
    for (id, sev) in [("fm-a", "data_loss"), ("fm-b", "recoverable_error")] {
        call(
            srv,
            "append",
            json!({
                "variant": "add_failure_mode",
                "session_id": SESSION,
                "failure_mode": {
                    "id": id,
                    "session_id": SESSION,
                    "component": { "id": "c:gateway" },
                    "description": "request dropped under load",
                    "cause": "unbounded queue",
                    "effect": "lost work",
                    "severity_observations": [sev],
                    "probability_observations": ["happens_in_normal_use"],
                    "domain": "runtime",
                    "source": { "turn_id": "t1" }
                }
            }),
        );
    }
    call(
        srv,
        "append",
        json!({
            "variant": "add_mitigation",
            "session_id": SESSION,
            "mitigation": {
                "id": "m1",
                "session_id": SESSION,
                "failure_mode_id": "fm-a",
                "kind": "prevention",
                "description": "bounded queue with backpressure",
                "residual_severity_observations": ["cosmetic"],
                "residual_probability_observations": ["rare_edge_case"],
                "source": { "turn_id": "t2" }
            }
        }),
    );
}

/// Every deterministic read of the session plus the raw log bytes.
fn snapshot(srv: &FmecaServer, dir: &Path) -> (Vec<Value>, Vec<u8>) {
    let reads = [
        "state.get",
        "risk.next",
        "readiness.assess",
        "report.export",
    ]
    .iter()
    .map(|tool| call(srv, tool, json!({ "session_id": SESSION })))
    .collect();
    let log = std::fs::read(dir.join(format!("{SESSION}.jsonl"))).unwrap();
    (reads, log)
}

/// A judge reporting a fixed cost.
struct CostlyJudge;

#[async_trait::async_trait]
impl JudgmentModel for CostlyJudge {
    async fn decide(
        &self,
        _state: Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<Decisions, Unavailable> {
        Ok(Decisions {
            answers: questions
                .iter()
                .map(|(id, q)| (id.clone(), answer(q, 0.5, 2.0, "prevention")))
                .collect(),
            model: "fake/costly".to_string(),
            usage: None,
            cost: Some(0.0012),
        })
    }
}

// --- availability ------------------------------------------------------------

#[tokio::test]
async fn vet_without_key_returns_vet_unavailable_no_key() {
    let (srv, _d) = server();
    let result = srv.with_judge(None).vet(analysis(1, 1)).await.unwrap();
    assert_eq!(
        (result["status"].clone(), result["reason"].clone()),
        (json!("vet_unavailable"), json!("no_key"))
    );
}

#[tokio::test]
async fn vet_unavailable_is_not_an_mcp_error() {
    let (srv, _d) = judged(Arc::new(FakeJudge::failing(UnavailableReason::Http5xx)));
    let Value::Object(args) = analysis(1, 1) else {
        unreachable!()
    };
    let result = srv
        .call_tool_value(CallToolRequestParams::new(TOOL_VET).with_arguments(args))
        .await;
    assert!(result.is_ok_and(|v| v["reason"] == "http_5xx"));
}

#[tokio::test]
async fn vet_reports_bad_config_when_configuration_was_rejected() {
    let (srv, _d) = server();
    let bad = JevConfig::from_lookup(|name| {
        (name == fmeca_mcp::jev::JEV_TIMEOUT_ENV).then(|| "0".to_string())
    })
    .unwrap_err();
    let result = srv
        .with_judge_unavailable(Some(bad))
        .vet(analysis(1, 1))
        .await
        .unwrap();
    assert_eq!(result["reason"], "bad_config");
}

// --- purity --------------------------------------------------------------------

#[tokio::test]
async fn vet_leaves_session_outputs_byte_identical() {
    let (srv, dir) = judged(fake(0.9, 4.0, "fail_fast"));
    seeded_session(&srv);
    let before = snapshot(&srv, dir.path());
    srv.vet(json!({ "session_id": SESSION })).await.unwrap();
    assert_eq!(before, snapshot(&srv, dir.path()));
}

#[tokio::test]
async fn vet_coverage_returns_candidates_without_appending() {
    let (srv, dir) = judged(fake(0.5, 4.0, "prevention"));
    seeded_session(&srv);
    let log = dir.path().join(format!("{SESSION}.jsonl"));
    let before = std::fs::read(&log).unwrap().len();
    srv.vet(json!({ "session_id": SESSION, "checks": ["coverage"] }))
        .await
        .unwrap();
    assert_eq!(before, std::fs::read(&log).unwrap().len());
}

#[tokio::test]
async fn vet_skips_excluded_failure_modes() {
    let (srv, _d) = judged(fake(0.5, 2.0, "prevention"));
    seeded_session(&srv);
    srv.engine()
        .set_failure_mode_disposition(
            SESSION,
            fmeca::DispositionChange {
                session_id: SESSION.to_string(),
                failure_mode_id: "fm-b".to_string(),
                disposition: fmeca::Disposition::Excluded,
                rationale: "not credible: queue is bounded upstream".to_string(),
                approver: None,
                evidence: vec![fmeca::ResolutionEvidence {
                    kind: fmeca::EvidenceKind::Test,
                    reference: "tests::queue_bound".to_string(),
                    basis: None,
                }],
                at: None,
            },
        )
        .unwrap();
    let result = srv
        .vet(json!({ "session_id": SESSION, "checks": ["observations", "mitigations"] }))
        .await
        .unwrap();
    assert!(
        result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| !f["target"].as_str().unwrap().starts_with("fm-b/"))
    );
}

// --- cap and hashes --------------------------------------------------------------

#[tokio::test]
async fn vet_sends_at_most_max_questions() {
    let judge = fake(0.5, 2.0, "detection");
    let (srv, _d) = judged(judge.clone());
    srv.vet(analysis(60, 3)).await.unwrap();
    assert_eq!(judge.calls()[0].len(), 40);
}

#[tokio::test]
async fn vet_lists_unsent_questions_as_skipped() {
    let (srv, _d) = judged(fake(0.5, 2.0, "detection"));
    let fms: Vec<fmeca::AnalyzeInput> =
        serde_json::from_value(analysis(60, 3)["analysis"]["failure_modes"].clone()).unwrap();
    let subject = subject_from_analysis(MatrixStrategy::default(), &fms).unwrap();
    let planned = plan_questions(&subject, &CHECKS).len();
    let result = srv.vet(analysis(60, 3)).await.unwrap();
    assert_eq!(result["skipped"].as_array().unwrap().len(), planned - 40);
}

#[tokio::test]
async fn cap_never_starves_readiness_and_coverage() {
    let judge = fake(0.5, 2.0, "detection");
    let (srv, _d) = judged(judge.clone());
    let fms: Vec<fmeca::AnalyzeInput> =
        serde_json::from_value(analysis(60, 3)["analysis"]["failure_modes"].clone()).unwrap();
    let subject = subject_from_analysis(MatrixStrategy::default(), &fms).unwrap();
    let analysis_level: Vec<String> = plan_questions(&subject, &["coverage", "readiness"])
        .into_iter()
        .map(|p| p.id)
        .collect();
    srv.vet(analysis(60, 3)).await.unwrap();
    let sent = judge.calls()[0].clone();
    assert!(analysis_level.iter().all(|id| sent.contains_key(id)));
}

#[test]
fn analysis_level_questions_come_first() {
    let fms: Vec<fmeca::AnalyzeInput> =
        serde_json::from_value(analysis(3, 2)["analysis"]["failure_modes"].clone()).unwrap();
    let subject = subject_from_analysis(MatrixStrategy::default(), &fms).unwrap();
    assert_eq!(plan_questions(&subject, &CHECKS)[0].target, "readiness");
}

#[tokio::test]
async fn vet_with_zero_max_questions_skips_everything() {
    let judge = fake(0.5, 2.0, "detection");
    let (srv, _d) = judged(judge.clone());
    let mut args = analysis(2, 1);
    args["max_questions"] = json!(0);
    let result = srv.vet(args).await.unwrap();
    assert_eq!(
        (result["status"].clone(), result["questions_sent"].clone()),
        (json!("ok"), json!(0))
    );
}

#[tokio::test]
async fn vet_question_hashes_are_stable() {
    let first = fake(0.5, 2.0, "detection");
    let second = fake(0.5, 2.0, "detection");
    judged(first.clone()).0.vet(analysis(3, 2)).await.unwrap();
    judged(second.clone()).0.vet(analysis(3, 2)).await.unwrap();
    let ids = |judge: &FakeJudge| judge.calls()[0].keys().cloned().collect::<Vec<_>>();
    assert_eq!(ids(&first), ids(&second));
}

// --- argument validation -----------------------------------------------------------

fn is_invalid_params(result: Result<Value, rmcp::ErrorData>) -> bool {
    result.is_err_and(|e| e.code == ErrorCode::INVALID_PARAMS)
}

#[tokio::test]
async fn vet_rejects_both_session_and_analysis() {
    let (srv, _d) = judged(fake(0.5, 2.0, "detection"));
    let mut args = analysis(1, 1);
    args["session_id"] = json!(SESSION);
    assert!(is_invalid_params(srv.vet(args).await));
}

#[tokio::test]
async fn vet_rejects_neither_session_nor_analysis() {
    let (srv, _d) = judged(fake(0.5, 2.0, "detection"));
    assert!(is_invalid_params(srv.vet(json!({})).await));
}

#[tokio::test]
async fn vet_rejects_max_questions_above_hard_cap() {
    let (srv, _d) = judged(fake(0.5, 2.0, "detection"));
    let mut args = analysis(1, 1);
    args["max_questions"] = json!(201);
    assert!(is_invalid_params(srv.vet(args).await));
}

#[tokio::test]
async fn vet_rejects_unknown_check() {
    let (srv, _d) = judged(fake(0.5, 2.0, "detection"));
    let mut args = analysis(1, 1);
    args["checks"] = json!(["observations", "vibes"]);
    assert!(is_invalid_params(srv.vet(args).await));
}

// --- interpretation ----------------------------------------------------------------

#[tokio::test]
async fn vet_flags_claimed_observation_below_threshold() {
    let (srv, _d) = judged(fake(0.1, 2.0, "detection"));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    let f = finding(&result, "fm-00/user_facing_degradation");
    assert_eq!(
        (f["verdict"].clone(), f["threshold"].clone()),
        (json!("flag"), json!(0.3))
    );
}

#[tokio::test]
async fn vet_flags_unclaimed_observation_above_threshold() {
    let (srv, _d) = judged(fake(0.9, 2.0, "detection"));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    let f = finding(&result, "fm-00/data_loss");
    assert_eq!(
        (f["verdict"].clone(), f["claimed"].clone()),
        (json!("flag"), json!(false))
    );
}

#[tokio::test]
async fn vet_flags_weak_mitigation_score() {
    let (srv, _d) = judged(fake(0.5, 1.0, "detection"));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    assert_eq!(finding(&result, "fm-00/fm-00-m0")["verdict"], "flag");
}

#[tokio::test]
async fn vet_flags_mitigation_class_mismatch() {
    let (srv, _d) = judged(fake(0.5, 2.0, "prevention"));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    assert_eq!(finding(&result, "fm-00/fm-00-m0#class")["verdict"], "flag");
}

#[tokio::test]
async fn vet_flags_coverage_gap() {
    let (srv, _d) = judged(fake(0.5, 3.0, "detection"));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    assert_eq!(finding(&result, "coverage/concurrency")["verdict"], "flag");
}

#[tokio::test]
async fn vet_raises_readiness_concern_below_threshold() {
    let (srv, _d) = judged(fake(0.5, 1.0, "detection"));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    assert_eq!(finding(&result, "readiness")["verdict"], "concern");
}

#[tokio::test]
async fn vet_wrong_answer_type_is_bad_response() {
    let judge = Arc::new(FakeJudge::answering(|_, _| Answer::Noul { noul: 0.5 }));
    let (srv, _d) = judged(judge);
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    assert_eq!(result["reason"], "bad_response");
}

#[tokio::test]
async fn vet_reports_readiness_beside_gate() {
    let (srv, _d) = judged(fake(0.5, 2.0, "prevention"));
    seeded_session(&srv);
    let gate = call(&srv, "readiness.assess", json!({ "session_id": SESSION }));
    let result = srv.vet(json!({ "session_id": SESSION })).await.unwrap();
    assert_eq!(result["readiness_gate"], gate);
}

#[tokio::test]
async fn vet_reports_usage_cost() {
    let (srv, _d) = judged(Arc::new(CostlyJudge));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    assert_eq!(result["usage"]["cost"], json!(0.0012));
}

// --- tool surface ------------------------------------------------------------------

#[test]
fn vet_schema_has_no_top_level_one_of() {
    let vet = fmeca_mcp::tool_definitions()
        .into_iter()
        .find(|t| t.name == TOOL_VET)
        .unwrap();
    assert!(
        ["oneOf", "anyOf", "allOf"]
            .iter()
            .all(|k| !vet.input_schema.contains_key(*k))
    );
}

#[test]
fn tools_list_adds_vet_and_keeps_the_eight() {
    assert_eq!(
        TOOL_NAMES,
        [
            "session.open",
            "append",
            "state.get",
            "risk.next",
            "readiness.assess",
            "report.export",
            "scoring.catalog",
            "analyze",
            "vet"
        ]
    );
}

// --- live ----------------------------------------------------------------------------

/// Spends about a cent on OpenRouter. Run explicitly with
/// `cargo test -p fmeca-mcp --features test-fakes --test vet -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "live OpenRouter call; needs OPENROUTER_API_KEY and spends money"]
async fn vet_live_against_openrouter() {
    let config = JevConfig::from_env()
        .expect("valid config")
        .expect("OPENROUTER_API_KEY is set");
    let (srv, _d) = judged(Arc::new(OpenRouterJudge::new(&config)));
    let result = srv.vet(analysis(1, 1)).await.unwrap();
    eprintln!(
        "live vet: status={} model={} cost={} questions_sent={}",
        result["status"], result["model"], result["usage"]["cost"], result["questions_sent"]
    );
    assert_eq!(result["status"], "ok", "{result}");
}
