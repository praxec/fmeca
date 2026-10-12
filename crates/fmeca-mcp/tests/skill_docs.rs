//! The agent skills under the workspace `skills/` (#18): their front matter,
//! and every tool, field and level they name, checked against the real tool
//! surface. A skill that names a tool or field the server does not have
//! teaches agents to make calls that fail, so these checks are the contract
//! between the docs and the code.
//!
//! Conventions the checks rely on:
//! - a tool is named in inline code: `session.open`, `vet`;
//! - a field is named as `tool.field` in inline code, e.g.
//!   `session.open.level`, `vet.max_questions` or
//!   `append.set_mitigation_status.evidence` (the `append` variant comes
//!   before its field);
//! - a level is named in inline code (`technical_plan`) or as a JSON
//!   `"level": "<value>"`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fmeca::{AnalysisLevel, Engine, FilesystemStore};
use fmeca_mcp::{FmecaServer, tool_definitions};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

// --- the skills on disk -----------------------------------------------------

fn skills_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills")
}

/// `(directory name, SKILL.md path)` for every skill, sorted.
fn skill_files() -> Vec<(String, PathBuf)> {
    let mut skills: Vec<(String, PathBuf)> = fs::read_dir(skills_root())
        .expect("skills/ exists")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("SKILL.md").is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path().join("SKILL.md"),
            )
        })
        .collect();
    skills.sort();
    skills
}

/// Every file under `skills/` with one of `extensions`, sorted.
fn files_with(extensions: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir)
            .expect("readable dir")
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, extensions, out);
            } else if path
                .extension()
                .is_some_and(|e| extensions.iter().any(|x| e == *x))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&skills_root(), extensions, &mut out);
    out.sort();
    out
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The name shown in an offender: the path below `skills/`.
fn shown(path: &Path) -> String {
    path.strip_prefix(skills_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The front matter's `key: value` pairs and the body after it.
fn front_matter(text: &str) -> Option<(BTreeMap<String, String>, String)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    let fields = rest[..end]
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Some((fields, rest[end + "\n---\n".len()..].to_string()))
}

/// A front-matter value without its surrounding double quotes, and whether
/// it had them.
fn unquote(value: &str) -> (String, bool) {
    match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        Some(inner) => (inner.replace("\\\"", "\""), true),
        None => (value.to_string(), false),
    }
}

/// Markdown outside fenced code blocks.
fn prose(text: &str) -> String {
    let mut fenced = false;
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The contents of every fenced block whose info string is `lang`.
fn fenced_blocks(text: &str, lang: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    // Inside a block: whether it is in `lang`, and its lines so far.
    let mut open: Option<(bool, String)> = None;
    for line in text.lines() {
        match (open.take(), line.trim_start().strip_prefix("```")) {
            (None, Some(info)) => open = Some((info.trim() == lang, String::new())),
            (None, None) => {}
            (Some((wanted, block)), Some(_)) => {
                if wanted {
                    blocks.push(block);
                }
            }
            (Some((wanted, mut block)), None) => {
                block.push_str(line);
                block.push('\n');
                open = Some((wanted, block));
            }
        }
    }
    blocks
}

/// Inline code spans in the prose of `text`.
fn inline_code(text: &str) -> Vec<String> {
    prose(text)
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// A dotted identifier such as `session.open` or `vet.max_questions`.
fn is_dotted_ident(token: &str) -> bool {
    token.contains('.')
        && token
            .split('.')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
}

// --- the real tool surface --------------------------------------------------

fn tool_names() -> BTreeSet<String> {
    tool_definitions()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Every property name anywhere in a JSON schema, plus `append`'s variants.
fn schema_names(schema: &Value, out: &mut BTreeSet<String>) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Object(props)) = map.get("properties") {
                for (name, sub) in props {
                    out.insert(name.clone());
                    if name == "variant"
                        && let Some(Value::Array(variants)) = sub.get("enum")
                    {
                        out.extend(variants.iter().filter_map(Value::as_str).map(String::from));
                    }
                    schema_names(sub, out);
                }
            }
            if let Some(items) = map.get("items") {
                schema_names(items, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| schema_names(i, out)),
        _ => {}
    }
}

/// Every object key anywhere in a JSON value.
fn value_keys(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                out.insert(k.clone());
                value_keys(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| value_keys(i, out)),
        _ => {}
    }
}

/// The documented `vet` output fields (it needs a live judge to produce them).
const VET_OUTPUT_FIELDS: [&str; 19] = [
    "status",
    "reason",
    "detail",
    "findings",
    "check",
    "target",
    "question_hash",
    "probability",
    "threshold",
    "verdict",
    "claimed",
    "declared",
    "choice",
    "model",
    "usage",
    "cost",
    "skipped",
    "readiness_gate",
    "questions_sent",
];

fn call(server: &FmecaServer, name: &str, args: Value) -> Value {
    let Value::Object(map) = args else {
        panic!("call expects a JSON object")
    };
    server
        .dispatch_call(CallToolRequestParams::new(name.to_string()).with_arguments(map))
        .unwrap_or_else(|e| panic!("{name} failed: {e:?}"))
}

fn server(dir: &Path) -> FmecaServer {
    let store = FilesystemStore::new(dir).expect("store");
    FmecaServer::new(Engine::new(Arc::new(store))).with_state_dir(dir.to_path_buf())
}

/// The output keys of every synchronous tool, read from real responses over a
/// small session with one mitigated failure mode.
fn output_keys() -> BTreeMap<String, BTreeSet<String>> {
    let dir = tempfile::tempdir().expect("tempdir");
    let srv = server(dir.path());
    let s = "docs-implementation";
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    out.insert(
        "session.open".into(),
        call(
            &srv,
            "session.open",
            json!({"session_id": s, "level": "implementation", "aspect": "docs"}),
        ),
    );
    let fm = json!({
        "id": "fm-1", "session_id": s, "component": {"id": "c:api"},
        "description": "d", "cause": "c", "effect": "e", "local_effect": "l",
        "next_effect": "n", "single_point": true, "refs": ["path:src/lib.rs"],
        "severity_observations": ["data_loss"],
        "probability_observations": ["happens_in_normal_use"],
        "domain": "runtime", "source": {"turn_id": "t1"}
    });
    call(
        &srv,
        "append",
        json!({"variant": "add_failure_mode", "session_id": s, "failure_mode": fm}),
    );
    let mitigation = json!({
        "id": "m-1", "session_id": s, "failure_mode_id": "fm-1", "kind": "prevention",
        "description": "guard", "residual_severity_observations": ["cosmetic"],
        "residual_probability_observations": ["rare_edge_case"], "source": {"turn_id": "t2"}
    });
    call(
        &srv,
        "append",
        json!({"variant": "add_mitigation", "session_id": s, "mitigation": mitigation}),
    );
    out.insert(
        "append".into(),
        call(
            &srv,
            "append",
            json!({"variant": "set_mitigation_status", "session_id": s,
                   "mitigation_id": "m-1", "status": "verified",
                   "evidence": [{"kind": "test", "reference": "t", "basis": "test"}]}),
        ),
    );
    for tool in [
        "state.get",
        "risk.next",
        "readiness.assess",
        "report.export",
    ] {
        out.insert(tool.into(), call(&srv, tool, json!({"session_id": s})));
    }
    out.insert(
        "scoring.catalog".into(),
        call(&srv, "scoring.catalog", json!({})),
    );
    out.insert("session.list".into(), call(&srv, "session.list", json!({})));
    out.insert(
        "actions.suggest".into(),
        call(
            &srv,
            "actions.suggest",
            json!({"session_id": s, "policy": "safety-critical", "group_by": "tag"}),
        ),
    );
    out.insert("policy.list".into(), call(&srv, "policy.list", json!({})));
    out.insert(
        "analyze".into(),
        call(
            &srv,
            "analyze",
            serde_json::from_str(&one_shot_example()).expect("json"),
        ),
    );
    let mut keys: BTreeMap<String, BTreeSet<String>> = out
        .into_iter()
        .map(|(tool, value)| {
            let mut set = BTreeSet::new();
            value_keys(&value, &mut set);
            (tool, set)
        })
        .collect();
    keys.insert(
        "vet".into(),
        VET_OUTPUT_FIELDS.iter().map(|f| f.to_string()).collect(),
    );
    keys
}

/// For each tool, every name a `tool.field` reference may use: its input
/// schema's properties and its output's keys.
fn known_fields() -> BTreeMap<String, BTreeSet<String>> {
    let mut outputs = output_keys();
    tool_definitions()
        .into_iter()
        .map(|tool| {
            let mut names = outputs.remove(tool.name.as_ref()).unwrap_or_default();
            schema_names(&Value::Object((*tool.input_schema).clone()), &mut names);
            (tool.name.to_string(), names)
        })
        .collect()
}

/// The longest tool name that `token` is, or starts with followed by `.`.
fn tool_prefix<'a>(token: &str, tools: &'a BTreeSet<String>) -> Option<&'a String> {
    tools
        .iter()
        .filter(|t| token == t.as_str() || token.starts_with(&format!("{t}.")))
        .max_by_key(|t| t.len())
}

fn one_shot_example() -> String {
    read(&skills_root().join("fmeca/examples/one-shot-analyze.json"))
}

// --- levels -----------------------------------------------------------------

/// Every `AnalysisLevel`. The match fails to compile when a level is added,
/// so this list cannot fall behind the kernel.
fn all_levels() -> Vec<AnalysisLevel> {
    use AnalysisLevel::*;
    let all = vec![
        Concept,
        Conops,
        TechnicalPlan,
        ProjectPlan,
        Implementation,
        Verification,
    ];
    for level in &all {
        match level {
            Concept | Conops | TechnicalPlan | ProjectPlan | Implementation | Verification => {}
        }
    }
    all
}

fn wire(level: AnalysisLevel) -> String {
    serde_json::to_value(level)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .expect("a level serializes to a string")
}

/// The skill that covers `level`.
fn skill_for(level: &str) -> String {
    match level {
        "concept" | "conops" => "fmeca-concept".to_string(),
        other => format!("fmeca-{}", other.replace('_', "-")),
    }
}

/// The values of every JSON `"level": "<value>"` in `text`.
fn json_level_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    for (start, _) in text.match_indices("\"level\"") {
        let rest = text[start + "\"level\"".len()..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            values.push(rest[..end].to_string());
        }
    }
    values
}

// --- the checks -------------------------------------------------------------

#[test]
fn front_matter_name_matches_directory() {
    let offenders: Vec<String> = skill_files()
        .into_iter()
        .filter(|(dir, path)| {
            front_matter(&read(path)).and_then(|(fields, _)| fields.get("name").cloned())
                != Some(dir.clone())
        })
        .map(|(dir, _)| dir)
        .collect();
    assert!(
        offenders.is_empty(),
        "name differs from directory: {offenders:?}"
    );
}

#[test]
fn descriptions_say_when_to_use() {
    let mut offenders = Vec::new();
    for (dir, path) in skill_files() {
        let text = read(&path);
        let Some(raw) = front_matter(&text).and_then(|(f, _)| f.get("description").cloned()) else {
            offenders.push(format!("{dir}: no description"));
            continue;
        };
        let (description, quoted) = unquote(&raw);
        if !description.starts_with("Use when") {
            offenders.push(format!("{dir}: does not start with 'Use when'"));
        }
        if description.chars().count() > 1024 {
            offenders.push(format!("{dir}: longer than 1024 characters"));
        }
        if description.contains(": ") && !quoted {
            offenders.push(format!("{dir}: contains ': ' but is not quoted"));
        }
    }
    assert!(offenders.is_empty(), "bad descriptions: {offenders:?}");
}

#[test]
fn tool_names_exist() {
    let tools = tool_names();
    let nouns: BTreeSet<&str> = tools
        .iter()
        .map(|t| t.split('.').next().unwrap_or(t))
        .collect();
    let mut offenders = Vec::new();
    for path in files_with(&["md"]) {
        for token in inline_code(&read(&path)) {
            let names_a_tool =
                is_dotted_ident(&token) && nouns.contains(token.split('.').next().unwrap_or(""));
            if names_a_tool && tool_prefix(&token, &tools).is_none() {
                offenders.push(format!("{}: `{token}`", shown(&path)));
            }
        }
    }
    assert!(offenders.is_empty(), "unknown tools: {offenders:?}");
}

#[test]
fn tool_fields_exist() {
    let tools = tool_names();
    let fields = known_fields();
    let mut offenders = Vec::new();
    for path in files_with(&["md"]) {
        for token in inline_code(&read(&path)) {
            if !is_dotted_ident(&token) {
                continue;
            }
            let Some(tool) = tool_prefix(&token, &tools) else {
                continue;
            };
            let known = &fields[tool];
            let path_part = token[tool.len()..].trim_start_matches('.');
            let unknown: Vec<&str> = path_part
                .split('.')
                .filter(|seg| !seg.is_empty() && !known.contains(*seg))
                .collect();
            if !unknown.is_empty() {
                offenders.push(format!("{}: `{token}` ({unknown:?})", shown(&path)));
            }
        }
    }
    assert!(offenders.is_empty(), "unknown fields: {offenders:?}");
}

#[test]
fn levels_match_the_kernel() {
    let levels: BTreeSet<String> = all_levels().into_iter().map(wire).collect();
    let mut offenders = Vec::new();
    for path in files_with(&["md", "json"]) {
        let text = read(&path);
        for value in json_level_values(&text) {
            if !levels.contains(&value) {
                offenders.push(format!("{}: \"level\": \"{value}\"", shown(&path)));
            }
        }
        for token in inline_code(&text) {
            let near_miss = !levels.contains(&token)
                && levels.contains(&token.to_ascii_lowercase().replace([' ', '-'], "_"));
            if near_miss {
                offenders.push(format!("{}: `{token}`", shown(&path)));
            }
        }
    }
    assert!(offenders.is_empty(), "not an AnalysisLevel: {offenders:?}");
}

#[test]
fn every_skill_links_the_reference() {
    let offenders: Vec<String> = skill_files()
        .into_iter()
        .filter(|(dir, path)| {
            dir.starts_with("fmeca-") && !read(path).contains("](../fmeca/SKILL.md)")
        })
        .map(|(dir, _)| dir)
        .collect();
    assert!(
        offenders.is_empty(),
        "no link to ../fmeca/SKILL.md: {offenders:?}"
    );
}

#[test]
fn every_level_has_a_skill() {
    let mut offenders = Vec::new();
    for level in all_levels().into_iter().map(wire) {
        let path = skills_root().join(skill_for(&level)).join("SKILL.md");
        let names_it = path.is_file()
            && front_matter(&read(&path))
                .is_some_and(|(_, body)| body.contains(&format!("`{level}`")));
        if !names_it {
            offenders.push(format!("{level} -> {}", shown(&path)));
        }
    }
    assert!(
        offenders.is_empty(),
        "levels without a skill: {offenders:?}"
    );
}

#[test]
fn skill_bodies_fit_their_line_budgets() {
    let offenders: Vec<String> = skill_files()
        .into_iter()
        .filter_map(|(dir, path)| {
            let budget = match dir.as_str() {
                "fmeca-technical-plan"
                | "fmeca-project-plan"
                | "fmeca-implementation"
                | "fmeca-verification" => 120,
                _ => 150,
            };
            let lines = front_matter(&read(&path)).map_or(usize::MAX, |(_, b)| b.lines().count());
            (lines > budget).then(|| format!("{dir}: {lines} > {budget}"))
        })
        .collect();
    assert!(offenders.is_empty(), "over budget: {offenders:?}");
}

#[test]
fn relative_links_resolve() {
    let mut offenders = Vec::new();
    for path in files_with(&["md"]) {
        let text = prose(&read(&path));
        for (start, _) in text.match_indices("](") {
            let rest = &text[start + 2..];
            let Some(end) = rest.find(')') else { continue };
            let target = rest[..end].split('#').next().unwrap_or("");
            if target.is_empty() || target.contains("://") {
                continue;
            }
            let dir = path.parent().unwrap_or(Path::new("."));
            if !dir.join(target).exists() {
                offenders.push(format!("{}: {target}", shown(&path)));
            }
        }
    }
    assert!(offenders.is_empty(), "broken links: {offenders:?}");
}

/// `skills install` ships only `skills/`, so a relative link that leaves it
/// would be broken in an installed skill: use an absolute URL instead.
#[test]
fn relative_links_stay_inside_the_skills_dir() {
    let root = fs::canonicalize(skills_root()).expect("skills/ exists");
    let mut offenders = Vec::new();
    for path in files_with(&["md"]) {
        let text = prose(&read(&path));
        for (start, _) in text.match_indices("](") {
            let rest = &text[start + 2..];
            let Some(end) = rest.find(')') else { continue };
            let target = rest[..end].split('#').next().unwrap_or("");
            if target.is_empty() || target.contains("://") {
                continue;
            }
            let dir = path.parent().unwrap_or(Path::new("."));
            let inside = fs::canonicalize(dir.join(target)).is_ok_and(|t| t.starts_with(&root));
            if !inside {
                offenders.push(format!("{}: {target}", shown(&path)));
            }
        }
    }
    assert!(offenders.is_empty(), "links leaving skills/: {offenders:?}");
}

#[test]
fn fmeca_policy_skill_embeds_its_policy_template() {
    let embedded = fmeca_mcp::skills::embedded();
    let shipped = embedded
        .iter()
        .find(|s| s.name == "fmeca-policy")
        .is_some_and(|s| {
            s.files
                .iter()
                .any(|(path, _)| path == "fmeca-policy/policy-template.toml")
        });
    assert!(shipped, "skills install must ship policy-template.toml");
}

#[test]
fn json_examples_parse() {
    let mut offenders = Vec::new();
    for path in files_with(&["md"]) {
        for block in fenced_blocks(&read(&path), "json") {
            if let Err(e) = serde_json::from_str::<Value>(&block) {
                offenders.push(format!("{}: {e}", shown(&path)));
            }
        }
    }
    for path in files_with(&["json"]) {
        if let Err(e) = serde_json::from_str::<Value>(&read(&path)) {
            offenders.push(format!("{}: {e}", shown(&path)));
        }
    }
    assert!(offenders.is_empty(), "invalid JSON: {offenders:?}");
}

#[test]
fn one_shot_example_is_accepted_by_analyze() {
    let dir = tempfile::tempdir().expect("tempdir");
    let map = serde_json::from_str(&one_shot_example()).expect("example parses");
    let result = server(dir.path())
        .dispatch_call(CallToolRequestParams::new("analyze".to_string()).with_arguments(map));
    assert!(result.is_ok(), "analyze rejected the example: {result:?}");
}
