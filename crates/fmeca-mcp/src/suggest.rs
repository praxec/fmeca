//! The read-only policy tools: `actions.suggest` and `policy.list`.
//!
//! Both read an analysis (or the policy files) and report; neither appends to
//! the log. fmeca suggests; it never acts.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fmeca::{AnalyzeInput, Engine, MatrixStrategy};
use fmeca_policy::{
    GroupBy, LoadedPolicy, PRESET_NAMES, PolicyError, evaluate, group, load_project_policy,
    parse_date, preset, resolve, subject_from_analysis, subject_from_state, valid_policy_name,
};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::{engine_error_to_mcp, parse_args, to_value};

/// The policy used when the caller names none.
pub const DEFAULT_POLICY: &str = "compat-0.1";

/// The project policies directory for a state dir: `<state_dir>/policies`
/// when the state dir is a project `.fmeca/` folder, else none.
pub fn project_policies_dir(state_dir: &Path) -> Option<PathBuf> {
    (state_dir
        .file_name()
        .is_some_and(|n| n == crate::PROJECT_DIR_NAME))
    .then(|| state_dir.join("policies"))
}

/// The inline analysis of `actions.suggest`: the `analyze` envelope.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineAnalysis {
    #[serde(default)]
    matrix_strategy: MatrixStrategy,
    failure_modes: Vec<AnalyzeInput>,
}

/// `actions.suggest` args (flat; exactly one of `session_id` / `analysis` is
/// enforced by the handler).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuggestArgs {
    session_id: Option<String>,
    analysis: Option<InlineAnalysis>,
    policy: Option<String>,
    as_of: Option<String>,
    bands: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    #[serde(default)]
    required_only: bool,
    group_by: Option<GroupBy>,
}

/// `policy.list` takes no arguments.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

fn policy_error(err: PolicyError) -> McpError {
    McpError::invalid_params(err.to_string(), None)
}

/// The server's current UTC calendar date, `YYYY-MM-DD`.
fn today_utc() -> String {
    time::OffsetDateTime::now_utc().date().to_string()
}

/// `actions.suggest`: evaluate a session or one-shot analysis under a policy.
///
/// `bands` keeps failure modes whose effective band is listed; `tags` and
/// `required_only` keep matching actions. Roll-ups and the sign-off verdict
/// always cover the whole analysis, never the filtered view.
pub fn actions_suggest(engine: &Engine, state_dir: &Path, args: Value) -> Result<Value, McpError> {
    let parsed: SuggestArgs = parse_args(args)?;
    let subject = match (parsed.session_id, parsed.analysis) {
        (Some(session_id), None) => {
            let state = engine.state(&session_id).map_err(engine_error_to_mcp)?;
            subject_from_state(&state)
        }
        (None, Some(analysis)) => {
            subject_from_analysis(analysis.matrix_strategy, &analysis.failure_modes)
                .map_err(engine_error_to_mcp)?
        }
        _ => {
            return Err(McpError::invalid_params(
                "pass exactly one of session_id and analysis",
                None,
            ));
        }
    };
    let name = parsed.policy.as_deref().unwrap_or(DEFAULT_POLICY);
    let project_dir = project_policies_dir(state_dir);
    let loaded = resolve(name, project_dir.as_deref()).map_err(policy_error)?;
    let as_of = parse_date(&parsed.as_of.unwrap_or_else(today_utc)).map_err(policy_error)?;
    let mut suggestions = evaluate(&subject, &loaded, as_of).map_err(policy_error)?;

    if let Some(bands) = &parsed.bands {
        suggestions.failure_modes.retain(|fm| {
            fm.effective_band
                .as_ref()
                .is_some_and(|b| bands.contains(b))
        });
    }
    let wanted: Option<BTreeSet<&String>> = parsed.tags.as_ref().map(|t| t.iter().collect());
    for fm in &mut suggestions.failure_modes {
        fm.actions.retain(|a| {
            (!parsed.required_only || a.required)
                && wanted
                    .as_ref()
                    .is_none_or(|w| a.tags.iter().any(|t| w.contains(t)))
        });
    }

    let mut out = to_value(&suggestions)?;
    if let (Some(by), Some(obj)) = (parsed.group_by, out.as_object_mut()) {
        obj.insert("groups".into(), to_value(&group(&suggestions, by))?);
    }
    Ok(out)
}

fn policy_row(loaded: &LoadedPolicy) -> Result<Value, McpError> {
    Ok(json!({
        "name": loaded.policy.name,
        "hash": loaded.hash,
        "class": loaded.policy.class,
        "description": loaded.policy.description,
        "source": to_value(&loaded.source)?,
    }))
}

/// `policy.list`: the built-in presets and the project's policy files. A
/// project file that fails to load is listed with its `error`.
pub fn policy_list(state_dir: &Path, args: Value) -> Result<Value, McpError> {
    let _: NoArgs = parse_args(args)?;
    let presets = PRESET_NAMES
        .iter()
        .filter_map(|name| preset(name))
        .map(|p| policy_row(&p))
        .collect::<Result<Vec<_>, _>>()?;

    let mut project = Vec::new();
    if let Some(dir) = project_policies_dir(state_dir) {
        let mut stems: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        stems.sort();
        for stem in stems {
            let loaded = if valid_policy_name(&stem) {
                load_project_policy(&dir, &stem)
            } else {
                Err(PolicyError::NotFound(format!(
                    "{stem}: not a valid policy name"
                )))
            };
            project.push(match loaded {
                Ok(p) => policy_row(&p)?,
                Err(e) => json!({ "name": stem, "source": "project", "error": e.to_string() }),
            });
        }
    }
    Ok(json!({ "presets": presets, "project": project }))
}
