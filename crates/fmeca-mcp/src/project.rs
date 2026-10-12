//! The `.fmeca/` project store: `init` scaffolds it, `check` validates it.
//!
//! One analysis is one append-only `<session_id>.jsonl` event log. `init`
//! writes the README and a `.gitattributes` that sets `merge=union` on those
//! logs; `check` replays every log and runs [`fmeca::validate_log`] over it so
//! a bad merge is caught before it is trusted.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fmeca::{FilesystemStore, LogProblem, LogProblemKind, StateStore, validate_log};

use crate::config::PROJECT_DIR_NAME;

/// Exact `.gitattributes` content. `merge=union` keeps both branches' events;
/// `check` then catches the duplicate ids a union merge can produce.
pub const GITATTRIBUTES: &str = "\
# fmeca analyses: append-only event logs. Union merge keeps both branches' events;
# run `fmeca-mcp check` after merging to catch duplicate ids.
*.jsonl text eol=lf merge=union
";

/// README explaining how the folder is used.
pub const README: &str = "\
# FMECA project store

This directory holds this project's FMECA analyses.

## Layout

Each analysis is one append-only event log named `<session_id>.jsonl`.
Name each session `<aspect>-<level>`, for example `checkout-technical-plan`.

The six analysis levels:

- `concept` — is this worth building?
- `conops` — how will the business run it?
- `technical_plan` — how will we build it?
- `project_plan` — how will we deliver it?
- `implementation` — what can fail in the code?
- `verification` — what did we prove?

## Rules

Logs are append-only. Never edit them by hand: make every change through the
MCP `append` tool so the event stream stays well formed.

Run `fmeca-mcp check` after merges and in CI. A union merge can keep both
branches' events and duplicate an id; `check` names the duplicate.

Resume an analysis with `session.open` on the same `<session_id>`.
";

/// What `init` did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    /// The file did not exist and was written.
    Created,
    /// The file already existed and was left exactly as it was.
    Unchanged,
}

/// Create `<project_root>/.fmeca/` with the README and `.gitattributes`.
///
/// Existing files are never overwritten: they are reported [`FileOutcome::Unchanged`]
/// even when their content differs. Returns one `(path, outcome)` per file.
pub fn init(project_root: &Path) -> io::Result<Vec<(PathBuf, FileOutcome)>> {
    let dir = project_root.join(PROJECT_DIR_NAME);
    fs::create_dir_all(&dir)?;

    let mut outcomes = Vec::new();
    for (name, content) in [("README.md", README), (".gitattributes", GITATTRIBUTES)] {
        let path = dir.join(name);
        // `create_new` fails on any existing entry, a dangling symlink included,
        // so init never writes through a link.
        let outcome = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                FileOutcome::Created
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => FileOutcome::Unchanged,
            Err(error) => return Err(error),
        };
        outcomes.push((path, outcome));
    }
    Ok(outcomes)
}

/// The result of checking one session log. Empty `problems` means `ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckLine {
    pub session_id: String,
    pub problems: Vec<String>,
}

/// Replay and validate every `<session_id>.jsonl` in `state_dir`, sorted by id.
///
/// A missing directory yields no lines; the CLI reports it separately.
pub fn check(state_dir: &Path) -> Vec<CheckLine> {
    if !state_dir.is_dir() {
        return Vec::new();
    }
    let Ok(store) = FilesystemStore::new(state_dir) else {
        return Vec::new();
    };
    let Ok(ids) = store.list_sessions() else {
        return Vec::new();
    };

    let mut lines = Vec::new();
    for id in ids {
        let problems = match store.replay(&id) {
            Ok(events) => validate_log(&id, &events)
                .into_iter()
                .map(format_problem)
                .collect(),
            Err(error) => vec![error.to_string()],
        };
        lines.push(CheckLine {
            session_id: id,
            problems,
        });
    }
    lines.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    lines
}

/// Format one [`LogProblem`] as `line <n>: <kind>: <detail>`.
fn format_problem(problem: LogProblem) -> String {
    format!(
        "line {}: {}: {}",
        problem.line,
        kind_name(problem.kind),
        problem.detail
    )
}

/// The stable snake_case wire name of a problem kind.
fn kind_name(kind: LogProblemKind) -> &'static str {
    match kind {
        LogProblemKind::MissingSessionOpened => "missing_session_opened",
        LogProblemKind::DuplicateSessionOpened => "duplicate_session_opened",
        LogProblemKind::DuplicateFailureModeId => "duplicate_failure_mode_id",
        LogProblemKind::DuplicateMitigationId => "duplicate_mitigation_id",
        LogProblemKind::UnknownFailureMode => "unknown_failure_mode",
        LogProblemKind::UnknownMitigation => "unknown_mitigation",
        LogProblemKind::SessionIdMismatch => "session_id_mismatch",
        // `LogProblemKind` is `#[non_exhaustive]`; a kind added to the kernel
        // later still prints until it gets its own name here.
        _ => "log_problem",
    }
}
