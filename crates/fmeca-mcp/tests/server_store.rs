//! The server surface for the `.fmeca/` project store (#29): analysis
//! metadata on `session.open`, the `session.list` tool, and resolution
//! tracking through the `set_mitigation_status` and
//! `set_failure_mode_disposition` `append` variants. Every test drives the
//! transport-free `dispatch_call` against a tempdir store; none touches the
//! network.

use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::{FmecaServer, TOOL_SESSION_LIST};
use rmcp::model::{CallToolRequestParams, ErrorCode};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

// --- fixtures ----------------------------------------------------------------

fn server() -> (FmecaServer, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FilesystemStore::new(dir.path()).unwrap();
    let engine = Engine::new(Arc::new(store));
    (
        FmecaServer::new(engine).with_state_dir(dir.path().to_path_buf()),
        dir,
    )
}

fn call(srv: &FmecaServer, name: &str, args: Value) -> Result<Value, rmcp::ErrorData> {
    let Value::Object(map) = args else {
        panic!("call expects a JSON object")
    };
    srv.dispatch_call(CallToolRequestParams::new(name.to_string()).with_arguments(map))
}

fn ok(srv: &FmecaServer, name: &str, args: Value) -> Value {
    call(srv, name, args).unwrap()
}

fn is_invalid_params(result: Result<Value, rmcp::ErrorData>) -> bool {
    result.is_err_and(|e| e.code == ErrorCode::INVALID_PARAMS)
}

/// A High-criticality failure mode `id` (data loss, happens in normal use).
fn add_high_fm(srv: &FmecaServer, session: &str, id: &str, extra: Value) -> Value {
    let mut fm = json!({
        "id": id,
        "session_id": session,
        "component": { "id": "c:gateway" },
        "description": "request dropped under load",
        "cause": "unbounded queue",
        "effect": "lost work",
        "severity_observations": ["data_loss"],
        "probability_observations": ["happens_in_normal_use"],
        "domain": "runtime",
        "source": { "turn_id": "t1" }
    });
    if let (Value::Object(fm), Value::Object(extra)) = (&mut fm, extra) {
        fm.extend(extra);
    }
    ok(
        srv,
        "append",
        json!({ "variant": "add_failure_mode", "session_id": session, "failure_mode": fm }),
    )
}

/// A prevention mitigation `id` on `fm` that drives the residual to Low.
fn add_low_mitigation(srv: &FmecaServer, session: &str, fm: &str, id: &str) -> Value {
    ok(
        srv,
        "append",
        json!({
            "variant": "add_mitigation",
            "session_id": session,
            "mitigation": {
                "id": id,
                "session_id": session,
                "failure_mode_id": fm,
                "kind": "prevention",
                "description": "bounded queue with backpressure",
                "residual_severity_observations": ["cosmetic"],
                "residual_probability_observations": ["rare_edge_case"],
                "source": { "turn_id": "t2" }
            }
        }),
    )
}

/// An open session `id` with one High failure mode mitigated to Low.
fn mitigated_session(srv: &FmecaServer, id: &str, meta: Value) {
    let mut args = json!({ "session_id": id });
    if let (Value::Object(args), Value::Object(meta)) = (&mut args, meta) {
        args.extend(meta);
    }
    ok(srv, "session.open", args);
    add_high_fm(srv, id, "fm1", json!({}));
    add_low_mitigation(srv, id, "fm1", "m1");
}

fn set_status(srv: &FmecaServer, session: &str, status: &str, evidence: Value) -> Value {
    ok(
        srv,
        "append",
        json!({
            "variant": "set_mitigation_status",
            "session_id": session,
            "mitigation_id": "m1",
            "status": status,
            "evidence": evidence
        }),
    )
}

fn test_evidence() -> Value {
    json!([{ "kind": "test", "reference": "tests::queue_bound", "basis": "test" }])
}

fn list(srv: &FmecaServer, args: Value) -> Value {
    ok(srv, TOOL_SESSION_LIST, args)
}

fn row_ids(listing: &Value) -> Vec<String> {
    listing["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["session_id"].as_str().unwrap().to_string())
        .collect()
}

// --- session.open metadata -----------------------------------------------------

#[test]
fn session_open_records_level_and_aspect() {
    let (srv, _d) = server();
    let state = ok(
        &srv,
        "session.open",
        json!({ "session_id": "checkout-technical-plan", "level": "technical_plan", "aspect": "checkout" }),
    );
    assert_eq!(
        state["meta"],
        json!({ "level": "technical_plan", "aspect": "checkout" })
    );
}

#[test]
fn session_open_without_meta_matches_previous_output_shape() {
    let (srv, _d) = server();
    let state = ok(&srv, "session.open", json!({ "session_id": "plain" }));
    let before = [
        "session_id",
        "matrix_strategy",
        "matrix_scale",
        "failure_modes",
        "issues",
        "signals",
        "registry",
        "readiness",
        "scoring_catalog",
    ];
    let keys = state.as_object().unwrap();
    assert_eq!(
        (
            before.iter().all(|k| keys.contains_key(*k)),
            state["meta"].clone()
        ),
        (true, json!({}))
    );
}

// --- set_mitigation_status --------------------------------------------------------

#[test]
fn set_mitigation_status_moves_to_verified_with_evidence() {
    let (srv, _d) = server();
    mitigated_session(&srv, "s", json!({}));
    let state = set_status(&srv, "s", "verified", test_evidence());
    let m = &state["failure_modes"][0]["mitigations"][0];
    assert_eq!(
        (m["status"].clone(), m["evidence"].clone()),
        (json!("verified"), test_evidence())
    );
}

#[test]
fn set_mitigation_status_stamps_rfc3339_time() {
    let (srv, _d) = server();
    mitigated_session(&srv, "s", json!({}));
    let state = set_status(&srv, "s", "implemented", json!([]));
    let at = state["failure_modes"][0]["mitigations"][0]["history"][0]["at"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(OffsetDateTime::parse(&at, &Rfc3339).is_ok(), "at = {at:?}");
}

#[test]
fn verified_without_evidence_is_invalid_params() {
    let (srv, _d) = server();
    mitigated_session(&srv, "s", json!({}));
    let result = call(
        &srv,
        "append",
        json!({
            "variant": "set_mitigation_status",
            "session_id": "s",
            "mitigation_id": "m1",
            "status": "verified",
            "evidence": [{ "kind": "note", "reference": "looked fine" }]
        }),
    );
    assert!(is_invalid_params(result));
}

// --- set_failure_mode_disposition ----------------------------------------------------

#[test]
fn accepted_disposition_without_approver_is_invalid_params() {
    let (srv, _d) = server();
    mitigated_session(&srv, "s", json!({}));
    let result = call(
        &srv,
        "append",
        json!({
            "variant": "set_failure_mode_disposition",
            "session_id": "s",
            "failure_mode_id": "fm1",
            "disposition": "accepted",
            "rationale": "cost of a fix outweighs the risk"
        }),
    );
    assert!(is_invalid_params(result));
}

#[test]
fn excluded_disposition_removes_failure_mode_from_risk_next() {
    let (srv, _d) = server();
    ok(&srv, "session.open", json!({ "session_id": "s" }));
    add_high_fm(&srv, "s", "fm1", json!({}));
    ok(
        &srv,
        "append",
        json!({
            "variant": "set_failure_mode_disposition",
            "session_id": "s",
            "failure_mode_id": "fm1",
            "disposition": "excluded",
            "rationale": "not credible: the queue is bounded upstream",
            "evidence": [{ "kind": "file", "reference": "src/queue.rs", "basis": "design" }]
        }),
    );
    let next = ok(&srv, "risk.next", json!({ "session_id": "s" }));
    assert_eq!(next["risk"], Value::Null);
}

// --- failure-mode effect levels -------------------------------------------------------

#[test]
fn add_failure_mode_accepts_effect_levels_and_single_point() {
    let (srv, _d) = server();
    ok(&srv, "session.open", json!({ "session_id": "s" }));
    let state = add_high_fm(
        &srv,
        "s",
        "fm1",
        json!({ "local_effect": "queue grows", "next_effect": "gateway OOM", "single_point": true }),
    );
    let fm = &state["failure_modes"][0];
    assert_eq!(
        (
            fm["local_effect"].clone(),
            fm["next_effect"].clone(),
            fm["single_point"].clone()
        ),
        (json!("queue grows"), json!("gateway OOM"), json!(true))
    );
}

#[test]
fn readiness_lists_critical_items() {
    let (srv, _d) = server();
    ok(&srv, "session.open", json!({ "session_id": "s" }));
    add_high_fm(&srv, "s", "fm1", json!({ "single_point": true }));
    let readiness = ok(&srv, "readiness.assess", json!({ "session_id": "s" }));
    assert_eq!(readiness["critical_items"], json!(["fm1"]));
}

// --- session.list ---------------------------------------------------------------------

#[test]
fn session_list_returns_every_analysis_sorted() {
    let (srv, _d) = server();
    for id in ["zeta", "alpha", "mid"] {
        ok(&srv, "session.open", json!({ "session_id": id }));
    }
    assert_eq!(row_ids(&list(&srv, json!({}))), ["alpha", "mid", "zeta"]);
}

#[test]
fn session_list_filters_by_level() {
    let (srv, _d) = server();
    ok(
        &srv,
        "session.open",
        json!({ "session_id": "checkout-conops", "level": "conops", "aspect": "checkout" }),
    );
    ok(
        &srv,
        "session.open",
        json!({
            "session_id": "checkout-technical-plan",
            "level": "technical_plan",
            "aspect": "checkout",
            "parent": "checkout-conops"
        }),
    );
    let listing = list(&srv, json!({ "level": "technical_plan" }));
    assert_eq!(row_ids(&listing), ["checkout-technical-plan"]);
}

#[test]
fn session_list_reports_ready_verified() {
    let (srv, _d) = server();
    mitigated_session(&srv, "s", json!({}));
    set_status(&srv, "s", "verified", test_evidence());
    let row = &list(&srv, json!({}))["sessions"][0];
    assert_eq!(
        (row["ready"].clone(), row["ready_verified"].clone()),
        (json!(true), json!(true))
    );
}

#[test]
fn session_list_shows_broken_log_as_error_row() {
    let (srv, dir) = server();
    ok(&srv, "session.open", json!({ "session_id": "good" }));
    std::fs::write(dir.path().join("broken.jsonl"), "this is not json\n").unwrap();
    let rows = list(&srv, json!({}))["sessions"].clone();
    assert!(
        rows[0]["session_id"] == "broken" && rows[0]["error"].is_string(),
        "{rows}"
    );
}

#[test]
fn session_list_schema_has_no_top_level_one_of() {
    let tool = fmeca_mcp::tool_definitions()
        .into_iter()
        .find(|t| t.name == TOOL_SESSION_LIST)
        .unwrap();
    assert!(
        ["oneOf", "anyOf", "allOf"]
            .iter()
            .all(|k| !tool.input_schema.contains_key(*k))
    );
}

#[test]
fn the_eight_pinned_tools_are_still_listed() {
    let names: Vec<String> = fmeca_mcp::tool_definitions()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    let pinned = [
        "session.open",
        "append",
        "state.get",
        "risk.next",
        "readiness.assess",
        "report.export",
        "scoring.catalog",
        "analyze",
    ];
    assert!(
        pinned.iter().all(|p| names.iter().any(|n| n == p)),
        "{names:?}"
    );
}
