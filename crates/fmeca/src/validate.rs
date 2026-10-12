//! `validate_log`: structural checks over a session's event log.
//!
//! The engine enforces these invariants at write time, but a log can still go
//! wrong outside it: a git union-merge of two branches that both appended to
//! the same `.jsonl`, a hand edit, or a copy between sessions. [`validate_log`]
//! reports such problems without failing; [`crate::projection::project`] stays
//! total over the same log (it ignores changes that name unknown ids).
//!
//! References are checked in log order: an event may only name a failure mode
//! or mitigation added on an earlier line. Ids share one namespace across
//! failure modes and mitigations, matching the engine's `DUPLICATE_ID` rule.
//! Cross-session `parent` links are not checked here (they need the store).

use std::collections::HashSet;

use serde::Serialize;

use crate::event::Event;

/// One problem found in a log. `line` is the 1-based position of the event
/// (0 for an empty log).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogProblem {
    pub line: usize,
    pub kind: LogProblemKind,
    pub detail: String,
}

/// The kinds of log problem. A disposition naming an unknown failure mode is
/// `UnknownFailureMode`. A `session_opened` after the first one (two logs
/// concatenated, for example by a union merge) is `DuplicateSessionOpened`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogProblemKind {
    MissingSessionOpened,
    DuplicateSessionOpened,
    DuplicateFailureModeId,
    DuplicateMitigationId,
    UnknownFailureMode,
    UnknownMitigation,
    SessionIdMismatch,
}

/// Validate `events` as the log of `session_id`. An empty result means valid.
pub fn validate_log(session_id: &str, events: &[Event]) -> Vec<LogProblem> {
    let mut problems = Vec::new();
    let mut push = |line: usize, kind: LogProblemKind, detail: String| {
        problems.push(LogProblem { line, kind, detail });
    };

    if !matches!(events.first(), Some(Event::SessionOpened { .. })) {
        push(
            usize::from(!events.is_empty()),
            LogProblemKind::MissingSessionOpened,
            "the first event is not session_opened".to_string(),
        );
    }

    let mut ids: HashSet<&str> = HashSet::new();
    let mut failure_modes: HashSet<&str> = HashSet::new();
    let mut mitigations: HashSet<&str> = HashSet::new();
    let mut session_opened_seen = false;

    for (idx, event) in events.iter().enumerate() {
        let line = idx + 1;
        if event.session_id() != session_id {
            push(
                line,
                LogProblemKind::SessionIdMismatch,
                format!(
                    "event names session '{}', expected '{session_id}'",
                    event.session_id()
                ),
            );
        }
        match event {
            Event::SessionOpened { .. } => {
                if session_opened_seen {
                    push(
                        line,
                        LogProblemKind::DuplicateSessionOpened,
                        "session_opened appears again after the first one".to_string(),
                    );
                }
                session_opened_seen = true;
            }
            Event::FailureModeAdded { failure_mode } => {
                let id = failure_mode.id.as_str();
                if !ids.insert(id) {
                    push(
                        line,
                        LogProblemKind::DuplicateFailureModeId,
                        format!("failure mode id '{id}' was already used"),
                    );
                }
                failure_modes.insert(id);
            }
            Event::MitigationAdded { mitigation } => {
                let id = mitigation.id.as_str();
                if !ids.insert(id) {
                    push(
                        line,
                        LogProblemKind::DuplicateMitigationId,
                        format!("mitigation id '{id}' was already used"),
                    );
                }
                let fm = mitigation.failure_mode_id.as_str();
                if !failure_modes.contains(fm) {
                    push(
                        line,
                        LogProblemKind::UnknownFailureMode,
                        format!("mitigation '{id}' names unknown failure mode '{fm}'"),
                    );
                }
                mitigations.insert(id);
            }
            Event::Rescored { rescore } => {
                let fm = rescore.failure_mode_id.as_str();
                if !failure_modes.contains(fm) {
                    push(
                        line,
                        LogProblemKind::UnknownFailureMode,
                        format!("rescore names unknown failure mode '{fm}'"),
                    );
                }
            }
            Event::MitigationStatusChanged { change } => {
                let id = change.mitigation_id.as_str();
                if !mitigations.contains(id) {
                    push(
                        line,
                        LogProblemKind::UnknownMitigation,
                        format!("status change names unknown mitigation '{id}'"),
                    );
                }
            }
            Event::FailureModeDispositioned { change } => {
                let fm = change.failure_mode_id.as_str();
                if !failure_modes.contains(fm) {
                    push(
                        line,
                        LogProblemKind::UnknownFailureMode,
                        format!("disposition names unknown failure mode '{fm}'"),
                    );
                }
            }
        }
    }
    problems
}
