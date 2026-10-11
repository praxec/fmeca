//! Failure-mode disposition: open / accepted / excluded, and its effect on
//! readiness and `risk.next`.

mod common;

use common::{disposition_change, evidence, failure_mode, temp_engine};
use fmeca::Disposition::{Accepted, Excluded, Open};
use fmeca::Engine;
use fmeca::EvidenceKind::{Note, Test};
use fmeca::Level::High;

/// One unmitigated High/High failure mode `fm-1` (a readiness blocker).
fn unmitigated_session() -> (Engine, tempfile::TempDir) {
    let (engine, dir) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm-1", High, High))
        .unwrap();
    (engine, dir)
}

fn accept(engine: &Engine) {
    engine
        .set_failure_mode_disposition(
            "s",
            disposition_change("s", "fm-1", Accepted, Some("lead"), vec![]),
        )
        .unwrap();
}

fn exclude(engine: &Engine) {
    engine
        .set_failure_mode_disposition(
            "s",
            disposition_change("s", "fm-1", Excluded, None, vec![evidence(Test, "t::x")]),
        )
        .unwrap();
}

#[test]
fn failure_mode_starts_open() {
    let (engine, _d) = unmitigated_session();
    assert_eq!(
        engine.state("s").unwrap().failure_modes[0].disposition,
        Open
    );
}

#[test]
fn accepted_requires_approver() {
    let (engine, _d) = unmitigated_session();
    let result = engine
        .set_failure_mode_disposition("s", disposition_change("s", "fm-1", Accepted, None, vec![]));
    assert!(result.is_err());
}

#[test]
fn accepted_risk_does_not_block_readiness() {
    let (engine, _d) = unmitigated_session();
    accept(&engine);
    assert!(engine.readiness("s").unwrap().ready);
}

#[test]
fn accepted_risk_is_listed() {
    let (engine, _d) = unmitigated_session();
    accept(&engine);
    assert_eq!(engine.readiness("s").unwrap().accepted_risks, vec!["fm-1"]);
}

#[test]
fn excluded_requires_non_note_evidence() {
    let (engine, _d) = unmitigated_session();
    let result = engine.set_failure_mode_disposition(
        "s",
        disposition_change("s", "fm-1", Excluded, None, vec![evidence(Note, "n/a")]),
    );
    assert!(result.is_err());
}

#[test]
fn excluded_failure_mode_leaves_risk_next() {
    let (engine, _d) = unmitigated_session();
    exclude(&engine);
    assert!(engine.risk_next("s").unwrap().is_none());
}

#[test]
fn excluded_failure_mode_is_kept_in_state() {
    let (engine, _d) = unmitigated_session();
    exclude(&engine);
    assert_eq!(
        engine.state("s").unwrap().failure_modes[0].disposition,
        Excluded
    );
}

#[test]
fn reopening_restores_blocking() {
    let (engine, _d) = unmitigated_session();
    accept(&engine);
    engine
        .set_failure_mode_disposition("s", disposition_change("s", "fm-1", Open, None, vec![]))
        .unwrap();
    assert!(!engine.readiness("s").unwrap().ready);
}

#[test]
fn disposition_history_keeps_every_change() {
    let (engine, _d) = unmitigated_session();
    accept(&engine);
    engine
        .set_failure_mode_disposition("s", disposition_change("s", "fm-1", Open, None, vec![]))
        .unwrap();
    exclude(&engine);
    assert_eq!(
        engine.state("s").unwrap().failure_modes[0]
            .disposition_history
            .len(),
        3
    );
}

#[test]
fn excluded_failure_mode_raises_no_issues() {
    let (engine, _d) = unmitigated_session();
    exclude(&engine);
    assert!(engine.state("s").unwrap().issues.is_empty());
}

#[test]
fn excluded_failure_mode_raises_no_signals() {
    let (engine, _d) = unmitigated_session();
    exclude(&engine);
    assert!(engine.state("s").unwrap().signals.is_empty());
}
