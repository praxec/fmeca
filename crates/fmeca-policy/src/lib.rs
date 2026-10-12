// Restriction-category lint on production code only; tests may `unwrap`.
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! `fmeca-policy` — declarative risk policies for fmeca.
//!
//! A policy (spec §3) is a TOML document that re-colours the kernel's fixed
//! criticality matrices into the policy's own risk bands, sets a sign-off rule,
//! and lists suggested actions. This crate owns the model, TOML parsing,
//! validation, the canonical hash, and loading project policies from
//! `.fmeca/policies/<name>.toml`.
//!
//! It is pure: no network, no clock, and no filesystem access except
//! [`load_project_policy`]. The kernel (`fmeca`) never depends on it.
//!
//! # Layout
//!
//! - [`model`] — the policy document types.
//! - [`validate()`] — the spec §3 structural rules.
//! - [`load_project_policy`] — the one filesystem reader.

pub mod model;

mod load;
mod validate;

pub use load::{load_project_policy, valid_policy_name};
pub use model::{
    Action, Band, Conditions, Confidence, DispositionKind, Matrix, Policy, Precedence,
    SCHEMA_VERSION, Signoff,
};
pub use validate::{parse_days, validate};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// A parsed, validated policy with its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPolicy {
    /// The policy.
    pub policy: Policy,
    /// Lower-case hex sha256 of the policy's canonical JSON
    /// (see [`policy_hash`]).
    pub hash: String,
    /// Where the policy came from.
    pub source: PolicySource,
}

/// Where a policy came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    /// A built-in preset.
    Preset,
    /// A project file under `.fmeca/policies/`.
    Project,
}

/// Policy errors; each `Display` starts with its stable error code.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PolicyError {
    /// No policy by this name (also: an invalid name, or a path escaping the
    /// policies directory).
    #[error("POLICY_NOT_FOUND: {0}")]
    NotFound(String),
    /// The policy document is malformed; `field` is the key path.
    #[error("INVALID_POLICY: {field}: {reason}")]
    Invalid {
        /// Key path of the offending field, e.g. `bands[1].rank`.
        field: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The policy has no matrix for the analysis's matrix strategy.
    #[error("POLICY_STRATEGY_MISMATCH: {0}")]
    StrategyMismatch(String),
}

pub(crate) fn invalid(field: impl Into<String>, reason: impl Into<String>) -> PolicyError {
    PolicyError::Invalid {
        field: field.into(),
        reason: reason.into(),
    }
}

/// Parse a TOML policy document. Syntax errors, unknown keys and type errors
/// are [`PolicyError::Invalid`] naming the key path. Does not
/// [`validate()`] the result.
pub fn parse_policy(toml_text: &str) -> Result<Policy, PolicyError> {
    // Syntax first, so a syntax error reports its position, not a guessed key.
    if let Err(e) = toml_text.parse::<toml::Table>() {
        let reason = match e.span() {
            Some(span) => {
                let (line, column) = line_column(toml_text, span.start);
                format!("line {line}, column {column}: {}", e.message().trim())
            }
            None => e.message().trim().to_string(),
        };
        return Err(invalid("toml", reason));
    }
    toml::from_str(toml_text).map_err(|e| {
        let field = e
            .span()
            .map(|span| key_path(toml_text, span.start, span.end))
            .unwrap_or_else(|| "toml".to_string());
        invalid(field, e.message().trim())
    })
}

/// Lower-case hex sha256 of `serde_json::to_vec(policy)`. Field order is fixed
/// by the struct definitions and every map is a `BTreeMap`, so the hash
/// depends only on the policy's content, never on TOML key order or layout.
pub fn policy_hash(policy: &Policy) -> String {
    #[allow(clippy::expect_used)]
    let json = serde_json::to_vec(policy).expect("a Policy always serialises to JSON");
    Sha256::digest(json)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Best-effort key path (`bands[1].colour`) for the TOML error at
/// `start..end`: the enclosing `[table]` / `[[array]]` header, the inline
/// table key on the error's line (if any), and the key under the error.
fn key_path(src: &str, start: usize, end: usize) -> String {
    let start = start.min(src.len());
    let end = end.clamp(start, src.len());
    let before = &src[..start];
    // Through the end of the error's line: a span on a header line (a missing
    // field) belongs to that header's table.
    let line_end = src[start..].find('\n').map_or(src.len(), |i| start + i);

    // Enclosing header, counting `[[array]]` occurrences for the index.
    let mut header = String::new();
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for line in src[..line_end].lines() {
        if let Some(name) = table_header(line, "[[", "]]") {
            let n = counts.entry(name.clone()).or_insert(0);
            header = format!("{name}[{n}]");
            *n += 1;
        } else if let Some(name) = table_header(line, "[", "]") {
            header = name;
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if !header.is_empty() {
        parts.push(header);
    }
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line_head = &src[line_start..start];
    let spanned = src[start..end].trim().trim_matches('"');
    let is_key = !spanned.is_empty() && spanned.bytes().all(is_bare_key_byte);
    // `key = { …, <error> …}` or `key = <error>`: the line's leading key.
    if let Some((lead, rest)) = line_head.split_once('=') {
        let lead = lead.trim();
        if !lead.is_empty() && lead.bytes().all(|b| is_bare_key_byte(b) || b == b'.') {
            parts.push(lead.to_string());
            if is_key && rest.contains('{') {
                parts.push(spanned.to_string());
            }
        }
    } else if is_key {
        parts.push(spanned.to_string());
    }
    if parts.is_empty() {
        "toml".to_string()
    } else {
        parts.join(".")
    }
}

/// 1-based line and column of byte offset `at`.
fn line_column(src: &str, at: usize) -> (usize, usize) {
    let before = &src[..at.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rfind('\n')
        .map_or(before.len(), |i| before.len() - i - 1)
        + 1;
    (line, column)
}

/// `Some(name)` when `line` is a table header `open name close`.
fn table_header(line: &str, open: &str, close: &str) -> Option<String> {
    let line = line.split('#').next().unwrap_or("").trim();
    let name = line.strip_prefix(open)?.strip_suffix(close)?.trim();
    let bare = !name.is_empty()
        && !name.starts_with('[')
        && name.bytes().all(|b| is_bare_key_byte(b) || b == b'.');
    bare.then(|| name.to_string())
}

fn is_bare_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}
