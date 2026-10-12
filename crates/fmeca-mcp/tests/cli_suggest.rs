//! CLI `suggest` and `policy` subcommands: exit codes, output shapes.
//!
//! Each test runs the built binary (`CARGO_BIN_EXE_fmeca-mcp`) against a
//! temp-dir analysis or policy file; no network.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::FmecaServer;
use fmeca_policy::PRESET_NAMES;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

/// One scored, unmitigated Low failure mode: `compat-0.1` sign-off passes.
const PASSING_ANALYSIS: &str = r#"{
  "failure_modes": [
    {
      "id": "fm-low",
      "component": { "id": "c:svc" },
      "description": "cosmetic glitch",
      "cause": "cosmetic rendering bug",
      "effect": "visual glitch only",
      "domain": "runtime",
      "severity_observations": ["cosmetic"],
      "probability_observations": ["rare_edge_case"],
      "mitigations": []
    }
  ]
}"#;

/// One scored, unmitigated High failure mode: `compat-0.1` sign-off fails.
const FAILING_ANALYSIS: &str = r#"{
  "failure_modes": [
    {
      "id": "fm-high",
      "component": { "id": "c:svc" },
      "description": "data loss under normal use",
      "domain": "runtime",
      "severity_observations": ["data_loss"],
      "probability_observations": ["happens_in_normal_use"],
      "mitigations": []
    }
  ]
}"#;

/// Two scored, unmitigated failure modes that land in the `standard` preset's
/// `compensate` and `review` bands.
const TWO_BAND_ANALYSIS: &str = r#"{
  "failure_modes": [
    {
      "id": "fm-compensate",
      "component": { "id": "c:svc" },
      "description": "edge case degradation",
      "cause": "narrow edge case",
      "effect": "degraded output",
      "domain": "runtime",
      "severity_observations": ["data_loss"],
      "probability_observations": ["rare_edge_case"],
      "mitigations": []
    },
    {
      "id": "fm-review",
      "component": { "id": "c:svc" },
      "description": "data loss under normal use",
      "cause": "unbounded queue",
      "effect": "lost work",
      "domain": "runtime",
      "severity_observations": ["data_loss"],
      "probability_observations": ["happens_in_normal_use"],
      "mitigations": []
    }
  ]
}"#;

/// A minimal valid project policy named `team` (compat-0.1 shape).
const VALID_POLICY: &str = r#"
name = "team"
description = "Team policy"
class = "standard"
schema = 1

[[bands]]
id = "none"
label = "No action"
rank = 0
dispositions = ["accept"]

[[bands]]
id = "review"
label = "Review required"
rank = 1
dispositions = ["mitigate"]

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["none", "none", "review"],
  ["none", "review", "review"],
  ["review", "review", "review"],
]

[signoff]
max_residual_band = "none"
"#;

fn write(dir: &tempfile::TempDir, name: &str, contents: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).expect("write fixture");
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fmeca-mcp"))
        .args(args)
        .output()
        .expect("run fmeca-mcp")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf-8 stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf-8 stderr")
}

fn call(server: &FmecaServer, name: &str, args: Value) {
    let Value::Object(map) = args else {
        panic!("call expects a JSON object")
    };
    server
        .dispatch_call(CallToolRequestParams::new(name.to_string()).with_arguments(map))
        .unwrap_or_else(|error| panic!("{name} failed: {error:?}"));
}

/// Seed a passing (sign-off `pass`) session in `dir` and return its id.
fn seed_passing_session(dir: &Path) -> String {
    let store = FilesystemStore::new(dir).unwrap();
    let engine = Engine::new(Arc::new(store));
    let server = FmecaServer::new(engine).with_state_dir(dir.to_path_buf());
    let session = "sess-pass";
    call(&server, "session.open", json!({ "session_id": session }));
    call(
        &server,
        "append",
        json!({
            "variant": "add_failure_mode",
            "session_id": session,
            "failure_mode": {
                "id": "fm-low",
                "session_id": session,
                "component": { "id": "c:svc" },
                "description": "cosmetic glitch",
                "cause": "cosmetic rendering bug",
                "effect": "visual glitch only",
                "domain": "runtime",
                "severity_observations": ["cosmetic"],
                "probability_observations": ["rare_edge_case"],
                "source": { "turn_id": "t1" }
            }
        }),
    );
    session.to_string()
}

#[test]
fn suggest_passing_analysis_exits_0() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "analysis.json", PASSING_ANALYSIS);
    let output = run(&["suggest", "--analysis", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn suggest_failing_analysis_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "analysis.json", FAILING_ANALYSIS);
    let output = run(&["suggest", "--analysis", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn suggest_unknown_policy_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "analysis.json", FAILING_ANALYSIS);
    let output = run(&[
        "suggest",
        "--analysis",
        file.to_str().unwrap(),
        "--policy",
        "no-such-policy",
    ]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn suggest_markdown_has_band_headings() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "analysis.json", TWO_BAND_ANALYSIS);
    let output = run(&[
        "suggest",
        "--analysis",
        file.to_str().unwrap(),
        "--policy",
        "standard",
        "--format",
        "markdown",
    ]);
    let markdown = stdout(&output);
    assert!(
        markdown.find("## review").unwrap() < markdown.find("## compensate").unwrap(),
        "higher-rank `review` must precede `compensate`:\n{markdown}"
    );
}

#[test]
fn policy_validate_ok_prints_hash() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "team.toml", VALID_POLICY);
    let output = run(&["policy", "validate", file.to_str().unwrap()]);
    let policy = fmeca_policy::parse_policy(VALID_POLICY).unwrap();
    fmeca_policy::validate(&policy).unwrap();
    let hash = fmeca_policy::policy_hash(&policy);
    assert_eq!(
        stdout(&output).trim(),
        format!("ok {} {}", policy.name, hash)
    );
}

#[test]
fn policy_validate_invalid_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "broken.toml", "name = \"broken\"\n");
    let output = run(&["policy", "validate", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn policy_list_includes_presets() {
    let output = run(&["policy", "list"]);
    let out = stdout(&output);
    assert!(PRESET_NAMES.iter().all(|name| out.contains(name)));
}

#[test]
fn policy_list_marks_preset_shadowed_project_file() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join(".fmeca");
    std::fs::create_dir_all(state.join("policies")).unwrap();
    std::fs::write(
        state.join("policies/standard.toml"),
        "name = \"standard\"\n",
    )
    .unwrap();
    let output = run(&["policy", "list", "--dir", state.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        out.lines()
            .any(|line| line == "project standard - shadowed-by-preset")
    );
}

#[test]
fn suggest_on_session_id_exits_by_signoff() {
    let dir = tempfile::tempdir().unwrap();
    let session = seed_passing_session(dir.path());
    let output = run(&["suggest", &session, "--dir", dir.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn suggest_with_both_session_and_analysis_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "analysis.json", PASSING_ANALYSIS);
    let output = run(&["suggest", "sess-pass", "--analysis", file.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn policy_list_dir_without_value_exits_2() {
    let output = run(&["policy", "list", "--dir"]);
    assert!(stderr(&output).contains("requires a value"));
}

#[test]
fn suggest_format_given_twice_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(&dir, "analysis.json", PASSING_ANALYSIS);
    let output = run(&[
        "suggest",
        "--analysis",
        file.to_str().unwrap(),
        "--format",
        "json",
        "--format",
        "markdown",
    ]);
    assert_eq!(output.status.code(), Some(2));
}
