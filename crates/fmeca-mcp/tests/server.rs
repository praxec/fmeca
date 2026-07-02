//! MCP server façade tests: drive each of the six tools through the
//! transport-free `dispatch_call` entry point, asserting the wire shapes and the
//! kernel's stable error prefixes. Uses real `fmeca` wire shapes.

use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::{
    FmecaServer, TOOL_ANALYZE, TOOL_APPEND, TOOL_NAMES, TOOL_READINESS_ASSESS, TOOL_REPORT_EXPORT,
    TOOL_RISK_NEXT, TOOL_SCORING_CATALOG, TOOL_SESSION_OPEN, TOOL_STATE_GET,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};

fn server() -> (FmecaServer, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(dir.path()).unwrap();
    let engine = Engine::new(Arc::new(store));
    (FmecaServer::new(engine), dir)
}

fn call(srv: &FmecaServer, name: &str, args: Value) -> Result<Value, rmcp::ErrorData> {
    let map = match args {
        Value::Object(m) => m,
        _ => panic!("call expects a JSON object"),
    };
    let params = CallToolRequestParams::new(name.to_string()).with_arguments(map);
    srv.dispatch_call(params)
}

/// A fully-scored, High-criticality failure mode. Severity/probability come from
/// OBSERVATIONS: `data_loss`→High severity, `happens_in_normal_use`
/// →High probability ⇒ High per the fixed S×P matrix. The caller never supplies
/// a score.
fn high_failure_mode(session_id: &str) -> Value {
    json!({
        "variant": "add_failure_mode",
        "session_id": session_id,
        "failure_mode": {
            "id": "fm1",
            "session_id": session_id,
            "component": { "id": "c:gateway" },
            "description": "request dropped under load",
            "cause": "unbounded queue",
            "effect": "lost work",
            "severity_observations": ["data_loss"],
            "probability_observations": ["happens_in_normal_use"],
            "domain": "runtime",
            "source": { "turn_id": "t1" }
        }
    })
}

/// A prevention mitigation that drives residual all the way down to Low/Low.
fn low_residual_mitigation(session_id: &str) -> Value {
    json!({
        "variant": "add_mitigation",
        "session_id": session_id,
        "mitigation": {
            "id": "m1",
            "session_id": session_id,
            "failure_mode_id": "fm1",
            "kind": "prevention",
            "description": "bounded queue with backpressure",
            "residual_severity_observations": ["cosmetic"],
            "residual_probability_observations": ["rare_edge_case"],
            "source": { "turn_id": "t2" }
        }
    })
}

#[test]
fn lists_eight_tools() {
    // Original six command/query tools + the v2 scoring.catalog query + the
    // stateless analyze batch tool.
    assert_eq!(TOOL_NAMES.len(), 8);
    let (srv, _d) = server();
    let defs = fmeca_mcp::tool_definitions();
    assert_eq!(defs.len(), 8);
    // every advertised name is dispatchable
    for name in TOOL_NAMES {
        let _ = srv.engine();
        assert!(defs.iter().any(|t| t.name == *name));
    }
    // the scoring.catalog and analyze tools are present
    assert!(TOOL_NAMES.contains(&TOOL_SCORING_CATALOG));
    assert!(TOOL_NAMES.contains(&TOOL_ANALYZE));
}

#[test]
fn scoring_catalog_lists_criteria_and_session_state_embeds_it() {
    let (srv, _d) = server();
    // scoring.catalog takes no args and returns the fixed criteria.
    let cat = call(&srv, TOOL_SCORING_CATALOG, json!({})).unwrap();
    let criteria = cat["criteria"].as_array().unwrap();
    assert!(!criteria.is_empty());
    // It carries id/axis/level/description and includes the documented seeds.
    assert!(criteria
        .iter()
        .any(|c| c["id"] == "data_loss" && c["axis"] == "severity" && c["level"] == "high"));
    assert!(criteria
        .iter()
        .any(|c| c["id"] == "rare_edge_case" && c["axis"] == "probability" && c["level"] == "low"));

    // The catalog is also embedded in session state so the caller knows the vocab.
    let opened = call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "vocab" })).unwrap();
    assert!(!opened["scoring_catalog"].as_array().unwrap().is_empty());
}

#[test]
fn session_open_defaults_to_3x3_strategy() {
    let (srv, _d) = server();
    let opened = call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(opened["matrix_strategy"], "qualitative3x3");
    assert_eq!(opened["matrix_scale"].as_array().unwrap().len(), 3);
    // The active catalog is the 3×3 catalog (data_loss present).
    assert!(opened["scoring_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "data_loss"));
}

#[test]
fn session_open_selects_nasa_5x5_strategy_and_its_catalog() {
    let (srv, _d) = server();
    let opened = call(
        &srv,
        TOOL_SESSION_OPEN,
        json!({ "session_id": "nasa", "matrix_strategy": "nasa8004_5x5" }),
    )
    .unwrap();
    assert_eq!(opened["matrix_strategy"], "nasa8004_5x5");
    assert_eq!(opened["matrix_scale"].as_array().unwrap().len(), 5);
    let cat = opened["scoring_catalog"].as_array().unwrap();
    assert!(cat
        .iter()
        .any(|c| c["id"] == "loss_of_life_or_mission" && c["level_ordinal"] == 5));
    // No 3×3 id leaks into the NASA catalog.
    assert!(!cat.iter().any(|c| c["id"] == "data_loss"));

    // scoring.catalog can be queried per strategy too.
    let nasa_cat = call(
        &srv,
        TOOL_SCORING_CATALOG,
        json!({ "matrix_strategy": "nasa8004_5x5" }),
    )
    .unwrap();
    assert_eq!(nasa_cat["matrix_strategy"], "nasa8004_5x5");
    assert!(nasa_cat["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "near_certain"));
}

#[test]
fn nasa_observation_id_invalid_under_3x3_session_via_mcp() {
    let (srv, _d) = server();
    // Default (3×3) session.
    call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "q" })).unwrap();
    let bad = json!({
        "variant": "add_failure_mode",
        "session_id": "q",
        "failure_mode": {
            "id": "fm1",
            "session_id": "q",
            "component": { "id": "c:svc" },
            "description": "x",
            "cause": "c",
            "effect": "e",
            "severity_observations": ["loss_of_life_or_mission"],
            "probability_observations": ["happens_in_normal_use"],
            "domain": "runtime",
            "source": { "turn_id": "t1" }
        }
    });
    let err = call(&srv, TOOL_APPEND, bad).unwrap_err();
    assert!(err.message.starts_with("INVALID_OBSERVATION:"));
}

#[test]
fn nasa_session_collapses_5x5_to_lmh_criticality_via_mcp() {
    let (srv, _d) = server();
    call(
        &srv,
        TOOL_SESSION_OPEN,
        json!({ "session_id": "n", "matrix_strategy": "nasa8004_5x5" }),
    )
    .unwrap();
    let fm = json!({
        "variant": "add_failure_mode",
        "session_id": "n",
        "failure_mode": {
            "id": "fm1",
            "session_id": "n",
            "component": { "id": "c:svc" },
            "description": "catastrophic, near-certain",
            "cause": "c",
            "effect": "e",
            "severity_observations": ["loss_of_life_or_mission"],
            "probability_observations": ["near_certain"],
            "domain": "runtime",
            "source": { "turn_id": "t1" }
        }
    });
    let st = call(&srv, TOOL_APPEND, fm).unwrap();
    let fmv = &st["failure_modes"][0];
    // The 5×5 input collapses to the public L/M/H bucket: 5×5 ⇒ high.
    assert_eq!(fmv["criticality"], "high");
    // The derived level carries the 5-level label.
    assert_eq!(fmv["derived_severity"]["label"], "catastrophic");
    assert_eq!(fmv["derived_severity"]["ordinal"], 5);
}

#[test]
fn unknown_observation_id_is_invalid_observation() {
    let (srv, _d) = server();
    call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    let bad = json!({
        "variant": "add_failure_mode",
        "session_id": "s1",
        "failure_mode": {
            "id": "fm1",
            "session_id": "s1",
            "component": { "id": "c:gateway" },
            "description": "x",
            "cause": "c",
            "effect": "e",
            "severity_observations": ["not_a_catalog_id"],
            "probability_observations": ["happens_in_normal_use"],
            "domain": "runtime",
            "source": { "turn_id": "t1" }
        }
    });
    let err = call(&srv, TOOL_APPEND, bad).unwrap_err();
    assert!(err.message.starts_with("INVALID_OBSERVATION:"));
}

#[test]
fn response_class_is_surfaced_in_state_and_export() {
    let (srv, _d) = server();
    call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    // High criticality + runtime domain ⇒ restructure.
    let st = call(&srv, TOOL_APPEND, high_failure_mode("s1")).unwrap();
    assert_eq!(st["failure_modes"][0]["response_class"], "restructure");
    let doc = call(&srv, TOOL_REPORT_EXPORT, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(doc["rows"][0]["response_class"], "restructure");
}

#[test]
fn session_open_then_state_get() {
    let (srv, _d) = server();
    let opened = call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(opened["session_id"], "s1");
    // a fresh session has no failure modes and is not ready
    assert!(opened["failure_modes"].as_array().unwrap().is_empty());
    assert_eq!(opened["readiness"]["ready"], false);

    let state = call(&srv, TOOL_STATE_GET, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(state["session_id"], "s1");
    assert!(state["registry"]["component_ids"].is_array());
}

#[test]
fn append_failure_mode_then_mitigation_recomputes_state() {
    let (srv, _d) = server();
    call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();

    // Add a High, unmitigated failure mode.
    let st = call(&srv, TOOL_APPEND, high_failure_mode("s1")).unwrap();
    let fmv = &st["failure_modes"][0];
    // `FailureMode` fields are flattened onto the view.
    assert_eq!(fmv["id"], "fm1");
    assert_eq!(fmv["component"]["id"], "c:gateway");
    assert_eq!(fmv["criticality"], "high");
    assert_eq!(fmv["residual_criticality"], "high");
    assert_eq!(fmv["standing"], "unmitigated");
    assert!(fmv["mitigations"].as_array().unwrap().is_empty());
    // the component id landed in the registry
    assert_eq!(st["registry"]["component_ids"][0], "c:gateway");

    // Add a mitigation that drives residual down to Low.
    let st = call(&srv, TOOL_APPEND, low_residual_mitigation("s1")).unwrap();
    let fmv = &st["failure_modes"][0];
    // raw criticality unchanged, residual now Low, standing acceptable.
    assert_eq!(fmv["criticality"], "high");
    assert_eq!(fmv["residual_criticality"], "low");
    assert_eq!(fmv["standing"], "acceptable");
    assert_eq!(fmv["mitigations"][0]["id"], "m1");
    assert_eq!(fmv["mitigations"][0]["kind"], "prevention");
}

#[test]
fn risk_next_returns_highest_unmitigated_then_null() {
    let (srv, _d) = server();
    call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    call(&srv, TOOL_APPEND, high_failure_mode("s1")).unwrap();

    // unmitigated High → risk.next points at it
    let r = call(&srv, TOOL_RISK_NEXT, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(r["risk"]["id"], "fm1");
    assert_eq!(r["risk"]["criticality"], "high");

    // once mitigated to Low, nothing unmitigated remains
    call(&srv, TOOL_APPEND, low_residual_mitigation("s1")).unwrap();
    let r = call(&srv, TOOL_RISK_NEXT, json!({ "session_id": "s1" })).unwrap();
    assert!(r["risk"].is_null());
}

#[test]
fn readiness_and_export_reflect_blockers() {
    let (srv, _d) = server();
    call(&srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    // An unmitigated High failure mode blocks readiness on residual criticality.
    call(&srv, TOOL_APPEND, high_failure_mode("s1")).unwrap();

    let r = call(&srv, TOOL_READINESS_ASSESS, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(r["ready"], false);
    let blockers = r["blockers"].as_array().unwrap();
    assert!(!blockers.is_empty());
    assert!(blockers
        .iter()
        .any(|b| b.as_str().unwrap().starts_with("RESIDUAL:")));
    assert_eq!(r["by_criticality"]["high"], 1);

    let doc = call(&srv, TOOL_REPORT_EXPORT, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(doc["session_id"], "s1");
    assert_eq!(doc["ready"], false);
    assert_eq!(doc["rows"][0]["failure_mode_id"], "fm1");
    assert_eq!(doc["rows"][0]["component"], "c:gateway");
    assert!(!doc["blockers"].as_array().unwrap().is_empty());
    // v1 has no accept-risk move
    assert!(doc["accepted_risks"].as_array().unwrap().is_empty());

    // Mitigating to Low clears the blockers and flips readiness to ready.
    call(&srv, TOOL_APPEND, low_residual_mitigation("s1")).unwrap();
    let r = call(&srv, TOOL_READINESS_ASSESS, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(r["ready"], true);
    assert!(r["blockers"].as_array().unwrap().is_empty());
    let doc = call(&srv, TOOL_REPORT_EXPORT, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(doc["ready"], true);
}

#[test]
fn old_log_mitigation_line_replays_with_empty_residual_observations() {
    // Event-migration: an OLD `mitigation_added` log line carried
    // `residual_severity` / `residual_probability` LEVELS, not observations. With
    // the new observation fields `#[serde(default)]` and no deny_unknown_fields,
    // an old line deserializes — the (now-removed) level fields are ignored and
    // the residual observation vecs default to empty — so replay stays
    // deterministic. (Empty residual observations ⇒ the mitigation contributes no
    // derived residual ⇒ residual falls back to the raw criticality.)
    let old_line = serde_json::json!({
        "type": "mitigation_added",
        "mitigation": {
            "id": "m1",
            "session_id": "s1",
            "failure_mode_id": "fm1",
            "kind": "prevention",
            "description": "legacy mitigation",
            "residual_severity": "low",
            "residual_probability": "low",
            "source": { "turn_id": "t2" }
        }
    });
    let event: fmeca::Event =
        serde_json::from_value(old_line).expect("old log line must still deserialize");
    match event {
        fmeca::Event::MitigationAdded { mitigation } => {
            assert!(mitigation.residual_severity_observations.is_empty());
            assert!(mitigation.residual_probability_observations.is_empty());
        }
        other => panic!("expected MitigationAdded, got {other:?}"),
    }
}

#[test]
fn unknown_tool_is_invalid_params() {
    let (srv, _d) = server();
    let err = call(&srv, "nope.nope", json!({})).unwrap_err();
    assert!(err.message.contains("Unknown tool"));
}

#[test]
fn session_not_found_prefix_propagates() {
    let (srv, _d) = server();
    let err = call(&srv, TOOL_STATE_GET, json!({ "session_id": "ghost" })).unwrap_err();
    assert!(err.message.starts_with("SESSION_NOT_FOUND:"));
}

#[test]
fn bad_session_id_is_invalid_params_with_prefix() {
    let (srv, _d) = server();
    let err = call(
        &srv,
        TOOL_SESSION_OPEN,
        json!({ "session_id": "../escape" }),
    )
    .unwrap_err();
    assert!(err.message.starts_with("BAD_SESSION_ID:"));
}

#[test]
fn malformed_args_are_invalid_params() {
    let (srv, _d) = server();
    // missing required session_id
    let err = call(&srv, TOOL_STATE_GET, json!({ "wrong": "x" })).unwrap_err();
    assert!(err.message.contains("invalid arguments"));
}

#[test]
fn analyze_computes_a_full_report_in_one_call() {
    let (srv, _d) = server();
    // Stateless: no session.open needed. Mixed High (unmitigated) + a fully
    // mitigated High that drives residual to Low.
    let report = call(
        &srv,
        TOOL_ANALYZE,
        json!({
            "failure_modes": [
                {
                    "id": "fm-high",
                    "component": { "id": "c:gateway" },
                    "description": "request dropped under load",
                    "cause": "unbounded queue",
                    "effect": "lost work",
                    "domain": "runtime",
                    "severity_observations": ["data_loss"],
                    "probability_observations": ["happens_in_normal_use"],
                    "mitigations": []
                },
                {
                    "id": "fm-mitigated",
                    "component": { "id": "c:svc" },
                    "description": "retries storm",
                    "cause": "no backoff",
                    "effect": "cascade",
                    "domain": "runtime",
                    "severity_observations": ["data_loss"],
                    "probability_observations": ["happens_in_normal_use"],
                    "mitigations": [
                        {
                            "id": "m1",
                            "kind": "prevention",
                            "description": "exponential backoff",
                            "residual_severity_observations": ["cosmetic"],
                            "residual_probability_observations": ["rare_edge_case"]
                        }
                    ]
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(report["matrix_strategy"], "qualitative3x3");
    // both modes computed
    let fms = report["failure_modes"].as_array().unwrap();
    assert_eq!(fms.len(), 2);
    let high = fms.iter().find(|f| f["id"] == "fm-high").unwrap();
    assert_eq!(high["criticality"], "high");
    assert_eq!(high["residual_criticality"], "high");
    assert_eq!(high["standing"], "unmitigated");
    assert_eq!(high["response_class"], "restructure");
    let mit = fms.iter().find(|f| f["id"] == "fm-mitigated").unwrap();
    assert_eq!(mit["residual_criticality"], "low");
    assert_eq!(mit["standing"], "acceptable");

    // risk_ranking puts the High unmitigated first.
    assert_eq!(report["risk_ranking"][0], "fm-high");
    // not ready: the unmitigated High residual stands.
    assert_eq!(report["ready"], false);
    assert!(report["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b.as_str().unwrap().starts_with("RESIDUAL:")));
}

#[test]
fn analyze_honors_nasa_5x5_strategy() {
    let (srv, _d) = server();
    let report = call(
        &srv,
        TOOL_ANALYZE,
        json!({
            "matrix_strategy": "nasa8004_5x5",
            "failure_modes": [
                {
                    "id": "fm1",
                    "component": { "id": "c:svc" },
                    "description": "catastrophic, near-certain",
                    "cause": "c",
                    "effect": "e",
                    "domain": "runtime",
                    "severity_observations": ["loss_of_life_or_mission"],
                    "probability_observations": ["near_certain"]
                }
            ]
        }),
    )
    .unwrap();
    assert_eq!(report["matrix_strategy"], "nasa8004_5x5");
    // 5×5 collapses to the L/M/H bucket: (5,5) ⇒ high.
    assert_eq!(report["failure_modes"][0]["criticality"], "high");
}

#[test]
fn analyze_rejects_unknown_observation_id() {
    let (srv, _d) = server();
    let err = call(
        &srv,
        TOOL_ANALYZE,
        json!({
            "failure_modes": [
                {
                    "id": "fm1",
                    "component": { "id": "c:svc" },
                    "description": "x",
                    "cause": "c",
                    "effect": "e",
                    "domain": "runtime",
                    "severity_observations": ["not_a_catalog_id"],
                    "probability_observations": ["happens_in_normal_use"]
                }
            ]
        }),
    )
    .unwrap_err();
    assert!(err.message.starts_with("INVALID_OBSERVATION:"));
}

#[test]
fn analyze_empty_batch_is_not_ready() {
    let (srv, _d) = server();
    let report = call(&srv, TOOL_ANALYZE, json!({ "failure_modes": [] })).unwrap();
    assert_eq!(report["ready"], false);
    assert!(report["failure_modes"].as_array().unwrap().is_empty());
    assert!(report["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b.as_str().unwrap().starts_with("EMPTY:")));
}
