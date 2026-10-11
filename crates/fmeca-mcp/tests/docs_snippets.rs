//! The install docs (`docs/AGENT-INSTALL.md`, `README.md`) are followed by
//! agents, so every snippet in them has to be valid: configs parse, no secret
//! is spelled out, and the commands and level values they show are accepted by
//! the code they describe.

use std::path::{Path, PathBuf};

use fmeca::AnalysisLevel;
use fmeca_mcp::cli;
use serde_json::Value;

const KEY_VAR: &str = "OPENROUTER_API_KEY";
const KEY_REFERENCES: [&str; 2] = ["${OPENROUTER_API_KEY}", "${env:OPENROUTER_API_KEY}"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|err| panic!("cannot read {relative}: {err}"))
}

/// The documents whose snippets are checked.
fn documents() -> Vec<(&'static str, String)> {
    ["docs/AGENT-INSTALL.md", "README.md", "llms.txt"]
        .into_iter()
        .map(|name| (name, read(name)))
        .collect()
}

/// `(file, language, body)` for every fenced block in the checked documents.
fn fences() -> Vec<(&'static str, String, String)> {
    let mut found = Vec::new();
    for (name, text) in documents() {
        let mut open: Option<(String, String)> = None;
        for line in text.lines() {
            let trimmed = line.trim_start();
            match (&mut open, trimmed.strip_prefix("```")) {
                (None, Some(lang)) => open = Some((lang.trim().to_string(), String::new())),
                (Some(_), Some(_)) => {
                    if let Some((lang, body)) = open.take() {
                        found.push((name, lang, body));
                    }
                }
                (Some((_, body)), None) => {
                    body.push_str(line);
                    body.push('\n');
                }
                (None, None) => {}
            }
        }
    }
    found
}

fn json_snippets() -> Vec<(&'static str, String)> {
    fences()
        .into_iter()
        .filter(|(_, lang, _)| lang == "json")
        .map(|(name, _, body)| (name, body))
        .collect()
}

/// Every string stored under `key` anywhere in `value`.
fn values_of(key: &str, value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key
                    && let Some(text) = v.as_str()
                {
                    out.push(text.to_string());
                }
                values_of(key, v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| values_of(key, v, out)),
        _ => {}
    }
}

#[test]
fn json_snippets_parse() {
    let broken: Vec<String> = json_snippets()
        .into_iter()
        .filter_map(|(name, body)| {
            serde_json::from_str::<Value>(&body)
                .err()
                .map(|err| format!("{name}: {err}\n{body}"))
        })
        .collect();
    assert!(broken.is_empty(), "invalid JSON snippets: {broken:#?}");
}

#[test]
fn toml_snippets_parse() {
    let broken: Vec<String> = fences()
        .into_iter()
        .filter(|(_, lang, _)| lang == "toml")
        .filter_map(|(name, _, body)| {
            body.parse::<toml::Table>()
                .err()
                .map(|err| format!("{name}: {err}\n{body}"))
        })
        .collect();
    assert!(broken.is_empty(), "invalid TOML snippets: {broken:#?}");
}

#[test]
fn agent_install_has_json_and_toml_snippets() {
    let langs: Vec<String> = fences()
        .into_iter()
        .filter(|(name, _, _)| *name == "docs/AGENT-INSTALL.md")
        .map(|(_, lang, _)| lang)
        .collect();
    assert!(
        langs.iter().any(|l| l == "json") && langs.iter().any(|l| l == "toml"),
        "AGENT-INSTALL.md must show JSON and TOML configs, found {langs:?}"
    );
}

#[test]
fn no_document_contains_a_literal_openrouter_key() {
    let leaks: Vec<&str> = documents()
        .into_iter()
        .filter(|(_, text)| text.contains("sk-or-"))
        .map(|(name, _)| name)
        .collect();
    assert!(leaks.is_empty(), "literal key in {leaks:?}");
}

#[test]
fn json_snippets_pass_the_key_only_as_an_env_reference() {
    let mut values = Vec::new();
    for (_, body) in json_snippets() {
        if let Ok(parsed) = serde_json::from_str::<Value>(&body) {
            values_of(KEY_VAR, &parsed, &mut values);
        }
    }
    let literal: Vec<String> = values
        .into_iter()
        .filter(|v| !KEY_REFERENCES.contains(&v.as_str()))
        .collect();
    assert!(literal.is_empty(), "OPENROUTER_API_KEY set to {literal:?}");
}

#[test]
fn toml_snippets_never_assign_the_key_a_value() {
    let assigned: Vec<String> = fences()
        .into_iter()
        .filter(|(_, lang, _)| lang == "toml")
        .flat_map(|(_, _, body)| body.lines().map(str::to_string).collect::<Vec<_>>())
        .filter(|line| {
            line.trim_start().starts_with(KEY_VAR) || line.replace(' ', "").contains("KEY=\"")
        })
        .collect();
    assert!(assigned.is_empty(), "TOML assigns the key: {assigned:?}");
}

#[test]
fn shell_snippets_assign_the_key_only_from_a_substitution() {
    let assignment = format!("{KEY_VAR}=");
    let literal: Vec<String> = documents()
        .into_iter()
        .flat_map(|(name, text)| {
            text.lines()
                .filter_map(|line| {
                    let value = line.split_once(&assignment)?.1;
                    let value = value.trim_start_matches('"');
                    (!value.starts_with('$')).then(|| format!("{name}: {line}"))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(literal.is_empty(), "key assigned a literal: {literal:?}");
}

/// Every `--target <value>` in `text`, skipping `<placeholder>` words.
fn target_values(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    words
        .windows(2)
        .filter(|pair| pair[0] == "--target")
        .map(|pair| {
            pair[1]
                .trim_matches(|c: char| c == '`' || c == ',' || c == '.' || c == '"')
                .to_string()
        })
        .filter(|value| !value.starts_with('<'))
        .collect()
}

#[test]
fn agent_install_names_every_skills_target() {
    let targets = target_values(&read("docs/AGENT-INSTALL.md"));
    let missing: Vec<&str> = [
        "claude",
        "codex",
        "cursor",
        "copilot",
        "gemini",
        "agents-md",
    ]
    .into_iter()
    .filter(|t| !targets.iter().any(|found| found == t))
    .collect();
    assert!(missing.is_empty(), "no --target example for {missing:?}");
}

#[test]
fn every_target_in_agent_install_is_accepted_by_the_cli() {
    let rejected: Vec<String> = target_values(&read("docs/AGENT-INSTALL.md"))
        .into_iter()
        .filter(|target| {
            let args: Vec<String> = ["skills", "install", "--target", target, "--project", "."]
                .map(String::from)
                .to_vec();
            cli::parse(&args).is_err()
        })
        .collect();
    assert!(rejected.is_empty(), "CLI rejects --target {rejected:?}");
}

#[test]
fn every_level_in_a_json_snippet_is_an_analysis_level() {
    let mut levels = Vec::new();
    for (_, body) in json_snippets() {
        if let Ok(parsed) = serde_json::from_str::<Value>(&body) {
            values_of("level", &parsed, &mut levels);
        }
    }
    let unknown: Vec<String> = levels
        .into_iter()
        .filter(|level| {
            serde_json::from_value::<AnalysisLevel>(Value::from(level.clone())).is_err()
        })
        .collect();
    assert!(unknown.is_empty(), "not an AnalysisLevel: {unknown:?}");
}

#[test]
fn agent_install_shows_a_level_in_a_json_snippet() {
    let mut levels = Vec::new();
    for (name, body) in json_snippets() {
        if name != "docs/AGENT-INSTALL.md" {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<Value>(&body) {
            values_of("level", &parsed, &mut levels);
        }
    }
    assert!(!levels.is_empty(), "no level value is checked");
}
