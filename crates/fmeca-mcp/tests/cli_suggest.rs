//! CLI `suggest` and `policy` subcommands: exit codes, output shapes.
//!
//! Each test runs the built binary (`CARGO_BIN_EXE_fmeca-mcp`) against a
//! temp-dir analysis or policy file; no network.

use std::path::PathBuf;
use std::process::{Command, Output};

use fmeca_policy::PRESET_NAMES;

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
    let file = write(&dir, "analysis.json", FAILING_ANALYSIS);
    let output = run(&[
        "suggest",
        "--analysis",
        file.to_str().unwrap(),
        "--policy",
        "compat-0.1",
        "--format",
        "markdown",
    ]);
    assert!(stdout(&output).contains("## mitigate"));
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
