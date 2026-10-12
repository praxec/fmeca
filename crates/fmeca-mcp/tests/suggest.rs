//! `actions.suggest` and `policy.list`: read-only policy queries. Drives the
//! tools through the transport-free `dispatch_call` entry point against a
//! `.fmeca/` state dir in a tempdir.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::{
    FmecaServer, TOOL_ACTIONS_SUGGEST, TOOL_APPEND, TOOL_POLICY_LIST, TOOL_SESSION_OPEN,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

struct Fixture {
    srv: FmecaServer,
    state_dir: PathBuf,
    _tmp: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join(".fmeca");
    fs::create_dir_all(&state_dir).unwrap();
    let engine = Engine::new(Arc::new(FilesystemStore::new(&state_dir).unwrap()));
    let srv = FmecaServer::new(engine).with_state_dir(state_dir.clone());
    Fixture {
        srv,
        state_dir,
        _tmp: tmp,
    }
}

fn call(srv: &FmecaServer, name: &str, args: Value) -> Result<Value, rmcp::ErrorData> {
    let Value::Object(map) = args else {
        panic!("call expects a JSON object")
    };
    srv.dispatch_call(CallToolRequestParams::new(name.to_string()).with_arguments(map))
}

fn add_fm(f: &Fixture, session: &str, id: &str) {
    call(
        &f.srv,
        TOOL_APPEND,
        json!({
            "variant": "add_failure_mode",
            "session_id": session,
            "failure_mode": {
                "id": id,
                "session_id": session,
                "component": { "id": "c:gateway" },
                "description": "request dropped under load",
                "severity_observations": ["data_loss"],
                "probability_observations": ["happens_in_normal_use"],
                "domain": "runtime",
                "source": { "turn_id": "t1" }
            }
        }),
    )
    .unwrap();
}

fn session(f: &Fixture, ids: &[&str]) {
    call(&f.srv, TOOL_SESSION_OPEN, json!({ "session_id": "s1" })).unwrap();
    for id in ids {
        add_fm(f, "s1", id);
    }
}

fn analysis_fm(id: &str) -> Value {
    json!({
        "id": id,
        "component": { "id": "c:gateway" },
        "description": "request dropped under load",
        "severity_observations": ["data_loss"],
        "probability_observations": ["happens_in_normal_use"],
        "domain": "runtime"
    })
}

fn suggest(f: &Fixture, args: Value) -> Result<Value, rmcp::ErrorData> {
    call(&f.srv, TOOL_ACTIONS_SUGGEST, args)
}

fn policies_dir(f: &Fixture) -> PathBuf {
    let dir = f.state_dir.join("policies");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A valid project policy: the `standard` preset renamed.
fn write_project_policy(dir: &Path, name: &str) {
    let text = include_str!("../../fmeca-policy/presets/standard.toml")
        .replace("name = \"standard\"", &format!("name = \"{name}\""));
    fs::write(dir.join(format!("{name}.toml")), text).unwrap();
}

fn tool_schema(name: &str) -> serde_json::Map<String, Value> {
    fmeca_mcp::tool_definitions()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap()
        .input_schema
        .as_ref()
        .clone()
}

fn first_fm_actions(v: &Value) -> Vec<Value> {
    v["failure_modes"][0]["actions"].as_array().unwrap().clone()
}

#[test]
fn suggest_defaults_to_compat_policy() {
    let f = fixture();
    session(&f, &["fm1"]);
    let out = suggest(&f, json!({ "session_id": "s1" })).unwrap();
    assert_eq!(out["policy"]["name"], "compat-0.1");
}

#[test]
fn suggest_on_session_returns_failure_modes_in_id_order() {
    let f = fixture();
    session(&f, &["fm2", "fm1", "fm3"]);
    let out = suggest(&f, json!({ "session_id": "s1" })).unwrap();
    let ids: Vec<&str> = out["failure_modes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fm| fm["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["fm1", "fm2", "fm3"]);
}

#[test]
fn suggest_on_analysis_matches_session_result() {
    let f = fixture();
    session(&f, &["fm1"]);
    let from_session = suggest(&f, json!({ "session_id": "s1", "policy": "standard" })).unwrap();
    let from_analysis = suggest(
        &f,
        json!({ "analysis": { "failure_modes": [analysis_fm("fm1")] }, "policy": "standard" }),
    )
    .unwrap();
    assert_eq!(
        from_analysis["failure_modes"],
        from_session["failure_modes"]
    );
}

#[test]
fn suggest_rejects_both_session_and_analysis() {
    let f = fixture();
    session(&f, &["fm1"]);
    let err = suggest(
        &f,
        json!({ "session_id": "s1", "analysis": { "failure_modes": [] } }),
    )
    .unwrap_err();
    assert!(err.message.contains("exactly one"), "{}", err.message);
}

#[test]
fn suggest_rejects_neither_session_nor_analysis() {
    let f = fixture();
    let err = suggest(&f, json!({})).unwrap_err();
    assert!(err.message.contains("exactly one"), "{}", err.message);
}

#[test]
fn policy_name_with_path_is_not_found() {
    let f = fixture();
    session(&f, &["fm1"]);
    let err = suggest(
        &f,
        json!({ "session_id": "s1", "policy": "../../etc/passwd" }),
    )
    .unwrap_err();
    assert!(
        err.message.starts_with("POLICY_NOT_FOUND"),
        "{}",
        err.message
    );
}

#[test]
fn invalid_project_policy_is_invalid_params() {
    let f = fixture();
    session(&f, &["fm1"]);
    fs::write(
        policies_dir(&f).join("broken.toml"),
        "name = \"broken\"\nbogus = 1\n",
    )
    .unwrap();
    let err = suggest(&f, json!({ "session_id": "s1", "policy": "broken" })).unwrap_err();
    assert!(err.message.starts_with("INVALID_POLICY"), "{}", err.message);
}

#[test]
fn project_policy_is_used_when_present() {
    let f = fixture();
    session(&f, &["fm1"]);
    write_project_policy(&policies_dir(&f), "mine");
    let out = suggest(&f, json!({ "session_id": "s1", "policy": "mine" })).unwrap();
    assert_eq!(out["policy"]["name"], "mine");
}

#[test]
fn group_by_tag_groups_actions() {
    let f = fixture();
    session(&f, &["fm1"]);
    let out = suggest(
        &f,
        json!({ "session_id": "s1", "policy": "standard", "group_by": "tag" }),
    )
    .unwrap();
    assert_eq!(out["groups"]["design"], json!(["fm1"]));
}

#[test]
fn required_only_filters_suggested() {
    let f = fixture();
    session(&f, &["fm1"]);
    let all = suggest(&f, json!({ "session_id": "s1", "policy": "standard" })).unwrap();
    let only = suggest(
        &f,
        json!({ "session_id": "s1", "policy": "standard", "required_only": true }),
    )
    .unwrap();
    let expected: Vec<Value> = first_fm_actions(&all)
        .into_iter()
        .filter(|a| a["required"] == true)
        .collect();
    assert_eq!(first_fm_actions(&only), expected);
}

#[test]
fn as_of_is_echoed() {
    let f = fixture();
    session(&f, &["fm1"]);
    let out = suggest(&f, json!({ "session_id": "s1", "as_of": "2030-01-02" })).unwrap();
    assert_eq!(out["as_of"], "2030-01-02");
}

#[test]
fn as_of_defaults_to_the_server_utc_date() {
    let f = fixture();
    session(&f, &["fm1"]);
    let out = suggest(&f, json!({ "session_id": "s1" })).unwrap();
    let today = time::OffsetDateTime::now_utc().date().to_string();
    // A midnight rollover between the two reads would flake; both are UTC dates.
    assert_eq!(out["as_of"], today);
}

#[test]
fn bands_filter_keeps_only_matching_effective_bands() {
    let f = fixture();
    session(&f, &["fm1"]);
    let out = suggest(
        &f,
        json!({ "session_id": "s1", "policy": "standard", "bands": ["no-such-band"] }),
    )
    .unwrap();
    assert_eq!(out["failure_modes"], json!([]));
}

#[test]
fn suggest_never_appends_to_log() {
    let f = fixture();
    session(&f, &["fm1"]);
    let log = f.state_dir.join("s1.jsonl");
    let before = fs::read(&log).unwrap();
    suggest(&f, json!({ "session_id": "s1", "policy": "standard" })).unwrap();
    assert_eq!(fs::read(&log).unwrap(), before);
}

#[test]
fn unknown_session_is_session_not_found() {
    let f = fixture();
    let err = suggest(&f, json!({ "session_id": "nope" })).unwrap_err();
    assert!(
        err.message.starts_with("SESSION_NOT_FOUND"),
        "{}",
        err.message
    );
}

#[test]
fn suggest_schema_has_no_top_level_one_of() {
    let schema = tool_schema(TOOL_ACTIONS_SUGGEST);
    assert!(
        ["oneOf", "anyOf", "allOf"]
            .iter()
            .all(|k| !schema.contains_key(*k))
    );
}

#[test]
fn policy_list_lists_presets_and_project_files() {
    let f = fixture();
    write_project_policy(&policies_dir(&f), "mine");
    let out = call(&f.srv, TOOL_POLICY_LIST, json!({})).unwrap();
    let names = |key: &str| -> Vec<String> {
        out[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect()
    };
    let mut all = names("presets");
    all.extend(names("project"));
    let mut expected: Vec<String> = fmeca_policy::PRESET_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
    expected.push("mine".to_string());
    assert_eq!(all, expected);
}

#[test]
fn policy_list_reports_invalid_project_file_with_error() {
    let f = fixture();
    fs::write(policies_dir(&f).join("broken.toml"), "bogus = 1\n").unwrap();
    let out = call(&f.srv, TOOL_POLICY_LIST, json!({})).unwrap();
    assert!(
        out["project"][0]["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("INVALID_POLICY")),
        "{out}"
    );
}

#[test]
fn policy_list_without_project_dir_has_empty_project() {
    let f = fixture();
    let out = call(&f.srv, TOOL_POLICY_LIST, json!({})).unwrap();
    assert_eq!(out["project"], json!([]));
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
    assert!(pinned.iter().all(|p| names.iter().any(|n| n == p)));
}
