//! `validate_log`: structural checks over a session's event log.

mod common;

use common::{failure_mode, mitigation, status_change};
use fmeca::Level::{High, Low};
use fmeca::MitigationKind::Prevention;
use fmeca::MitigationStatus::Implemented;
use fmeca::{Event, LogProblemKind, MatrixStrategy, SessionMeta, validate_log};

fn opened() -> Event {
    Event::SessionOpened {
        session_id: "s".to_string(),
        matrix_strategy: MatrixStrategy::default(),
        meta: SessionMeta::default(),
    }
}

fn fm_added(id: &str) -> Event {
    Event::FailureModeAdded {
        failure_mode: Box::new(failure_mode("s", id, High, High)),
    }
}

fn mit_added(id: &str, fm_id: &str) -> Event {
    Event::MitigationAdded {
        mitigation: Box::new(mitigation("s", id, fm_id, Prevention, Low, Low)),
    }
}

fn kinds(events: &[Event]) -> Vec<LogProblemKind> {
    validate_log("s", events)
        .into_iter()
        .map(|p| p.kind)
        .collect()
}

#[test]
fn valid_log_has_no_problems() {
    let events = vec![opened(), fm_added("fm-1"), mit_added("m-1", "fm-1")];
    assert!(validate_log("s", &events).is_empty());
}

#[test]
fn duplicate_failure_mode_id_is_reported() {
    // The shape a git union-merge of two branches produces.
    let events = vec![opened(), fm_added("fm-1"), fm_added("fm-1")];
    assert_eq!(kinds(&events), vec![LogProblemKind::DuplicateFailureModeId]);
}

#[test]
fn mitigation_for_unknown_failure_mode_is_reported() {
    let events = vec![opened(), mit_added("m-1", "fm-x")];
    assert_eq!(kinds(&events), vec![LogProblemKind::UnknownFailureMode]);
}

#[test]
fn status_change_for_unknown_mitigation_is_reported() {
    let events = vec![
        opened(),
        fm_added("fm-1"),
        Event::MitigationStatusChanged {
            change: Box::new(status_change("s", "m-x", Implemented, vec![])),
        },
    ];
    assert_eq!(kinds(&events), vec![LogProblemKind::UnknownMitigation]);
}

#[test]
fn log_without_session_opened_is_reported() {
    let events = vec![fm_added("fm-1")];
    assert_eq!(kinds(&events), vec![LogProblemKind::MissingSessionOpened]);
}

#[test]
fn projection_ignores_status_change_for_unknown_mitigation() {
    let events = vec![
        opened(),
        fm_added("fm-1"),
        mit_added("m-1", "fm-1"),
        Event::MitigationStatusChanged {
            change: Box::new(status_change("s", "m-x", Implemented, vec![])),
        },
    ];
    let state = fmeca::projection::project(&events);
    assert_eq!(
        state.failure_modes[0].mitigations[0].status,
        fmeca::MitigationStatus::Planned
    );
}
