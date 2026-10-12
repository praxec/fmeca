//! Built-in policy presets and the shared action catalog (spec §6, plan
//! Task 3).
//!
//! Presets are compiled-in TOML documents ([`include_str!`]) parsed and
//! validated on demand. [`preset`] is panic-free: a built-in that does not
//! parse or validate is `None`, which the preset test-suite treats as a
//! failure. A preset that sets `include_builtin_actions` has the shared
//! [`builtin_actions`] catalog merged into its own actions.
//!
//! [`resolve`] is the public entry point used by the MCP and CLI layers: it
//! prefers a built-in preset, then a project file at
//! `<project_dir>/<name>.toml` (see
//! [`load_project_policy`](crate::load_project_policy)).

use std::path::Path;

use serde::Deserialize;

use crate::model::{Action, Policy};
use crate::{
    LoadedPolicy, PolicyError, PolicySource, load_project_policy, parse_policy, policy_hash,
    validate,
};

/// The names of the built-in presets, in the order they are listed in
/// `docs/policies.md`.
pub const PRESET_NAMES: [&str; 6] = [
    "compat-0.1",
    "prototype",
    "standard",
    "business-critical",
    "safety-critical",
    "confidence-view",
];

/// The compiled-in action catalog.
const ACTIONS_TOML: &str = include_str!("../presets/actions.toml");

/// The catalog TOML is a document holding an `actions` array.
#[derive(Deserialize)]
struct Catalog {
    actions: Vec<Action>,
}

/// The built-in action catalog (spec §6). Parsing is infallible for the
/// compiled-in document; a malformed catalog yields an empty list, which the
/// preset test-suite treats as a failure.
pub fn builtin_actions() -> Vec<Action> {
    match toml::from_str::<Catalog>(ACTIONS_TOML) {
        Ok(catalog) => catalog.actions,
        Err(_) => Vec::new(),
    }
}

/// The TOML of a built-in preset by name.
fn preset_text(name: &str) -> Option<&'static str> {
    Some(match name {
        "compat-0.1" => include_str!("../presets/compat-0.1.toml"),
        "prototype" => include_str!("../presets/prototype.toml"),
        "standard" => include_str!("../presets/standard.toml"),
        "business-critical" => include_str!("../presets/business-critical.toml"),
        "safety-critical" => include_str!("../presets/safety-critical.toml"),
        "confidence-view" => include_str!("../presets/confidence-view.toml"),
        _ => return None,
    })
}

/// Parse, validate and hash a built-in preset. `None` when `name` is not a
/// preset, or when a built-in is malformed (never a silent fallback).
///
/// When the preset sets `include_builtin_actions`, the shared
/// [`builtin_actions`] catalog is merged into its own actions (its own action
/// ids win), so the returned policy is self-contained. The
/// [evaluator](crate::evaluate) also merges the catalog at query time for
/// project policies.
pub fn preset(name: &str) -> Option<LoadedPolicy> {
    let mut policy = parse_policy(preset_text(name)?).ok()?;
    if policy.include_builtin_actions {
        for action in builtin_actions() {
            if !policy.actions.iter().any(|a| a.id == action.id) {
                policy.actions.push(action);
            }
        }
        policy
            .actions
            .sort_by(|a, b| (a.precedence, &a.id).cmp(&(b.precedence, &b.id)));
    }
    validate(&policy).ok()?;
    Some(LoadedPolicy {
        hash: policy_hash(&policy),
        policy,
        source: PolicySource::Preset,
    })
}

/// Resolve a policy name: a built-in preset first, then a project policy file
/// at `<project_dir>/<name>.toml` (when a project directory is supplied). An
/// unknown name, or one that escapes the project directory, is
/// [`PolicyError::NotFound`].
pub fn resolve(name: &str, project_dir: Option<&Path>) -> Result<LoadedPolicy, PolicyError> {
    if let Some(preset) = preset(name) {
        return Ok(preset);
    }
    match project_dir {
        Some(dir) => load_project_policy(dir, name),
        None => Err(PolicyError::NotFound(name.to_string())),
    }
}

/// The `standard` preset's policy, used by the evaluator to derive the row of
/// a `confidence` matrix. `None` only if the built-in is malformed, which the
/// preset test-suite rejects.
pub(crate) fn standard_policy() -> Option<Policy> {
    preset("standard").map(|loaded| loaded.policy)
}
