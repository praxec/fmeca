//! Readiness-gate table tests: ready iff every FM scored AND no
//! residual High/Medium AND no weak_mitigation_order standing.

mod common;

use common::{failure_mode, mitigation, temp_engine};
use fmeca::Level::{High, Low, Medium};
use fmeca::MitigationKind::{Detection, FailFast, Prevention};

#[test]
fn empty_session_is_not_ready() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(!r.ready);
    assert!(r.blockers.iter().any(|b| b.starts_with("EMPTY")));
}

#[test]
fn unscored_failure_mode_blocks_readiness() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = failure_mode("s", "fm", High, High);
    fm.cause = None;
    engine.add_failure_mode("s", fm).unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(!r.ready);
    assert!(r.blockers.iter().any(|b| b.starts_with("CLARIFY")));
}

#[test]
fn residual_high_blocks_readiness() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(!r.ready);
    assert!(r.blockers.iter().any(|b| b.starts_with("RESIDUAL")));
    assert_eq!(r.by_criticality.high, 1);
}

#[test]
fn residual_medium_blocks_readiness() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", Medium, Medium))
        .unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(!r.ready);
    assert_eq!(r.by_criticality.medium, 1);
}

#[test]
fn weak_mitigation_order_blocks_readiness_even_if_residual_low() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    // fail_fast-only mitigation that reduces residual to Low: residual is fine,
    // but the discipline blocker stands.
    engine
        .add_mitigation("s", mitigation("s", "m", "fm", FailFast, Low, Low))
        .unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(!r.ready);
    assert!(r.blockers.iter().any(|b| b.starts_with("DISCIPLINE")));
}

#[test]
fn fully_mitigated_to_low_is_ready() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    engine
        .add_mitigation("s", mitigation("s", "m", "fm", Prevention, Low, Low))
        .unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(r.ready, "blockers: {:?}", r.blockers);
    assert_eq!(r.by_criticality.low, 1);
    assert!(r.blockers.is_empty());
}

#[test]
fn low_criticality_failure_mode_needs_no_mitigation_to_be_ready() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    // Low/Low => Low criticality => acceptable with no mitigation.
    engine
        .add_failure_mode("s", failure_mode("s", "fm", Low, Low))
        .unwrap();
    let r = engine.readiness("s").unwrap();
    assert!(r.ready, "blockers: {:?}", r.blockers);
}

#[test]
fn mixed_set_blocks_until_all_reduced() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm1", High, High))
        .unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm2", Low, Low))
        .unwrap();
    // Mitigate fm1 properly (prevention to Low). fm2 already Low.
    let state = engine
        .add_mitigation("s", mitigation("s", "m1", "fm1", Detection, Low, Low))
        .unwrap();
    assert!(
        state.readiness.ready,
        "blockers: {:?}",
        state.readiness.blockers
    );
}
