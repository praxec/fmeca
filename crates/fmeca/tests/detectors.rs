//! Detector table tests: one test per [`IssueType`], plus
//! the signal mapping (notify / clarify / remediate).

mod common;

use common::{failure_mode, mitigation, temp_engine};
use fmeca::Level::{High, Low, Medium};
use fmeca::MitigationKind::{Detection, FailFast, Prevention};
use fmeca::{Domain, EntityRef, EvidenceRef, FailureMode, FmecaState, IssueType, SignalKind};

fn has_issue(state: &FmecaState, fm_id: &str, ty: IssueType) -> bool {
    state
        .issues
        .iter()
        .any(|i| i.failure_mode_id == fm_id && i.r#type == ty)
}

/// A failure mode with selected fields blanked, for the completeness detectors.
fn bare(session: &str, id: &str) -> FailureMode {
    FailureMode {
        id: id.to_string(),
        session_id: session.to_string(),
        component: EntityRef::new("comp:svc"),
        description: format!("fm {id}"),
        cause: Some("c".into()),
        effect: Some("e".into()),
        severity_observations: vec!["data_loss".into()],
        probability_observations: vec!["happens_in_normal_use".into()],
        domain: Domain::Runtime,
        scope: None,
        source: EvidenceRef::new("t1"),
    }
}

#[test]
fn missing_cause_detected() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = bare("s", "fm");
    fm.cause = None;
    let state = engine.add_failure_mode("s", fm).unwrap();
    assert!(has_issue(&state, "fm", IssueType::MissingCause));
}

#[test]
fn missing_effect_detected() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = bare("s", "fm");
    fm.effect = None;
    let state = engine.add_failure_mode("s", fm).unwrap();
    assert!(has_issue(&state, "fm", IssueType::MissingEffect));
}

#[test]
fn missing_score_detected() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = bare("s", "fm");
    fm.severity_observations = vec![];
    let state = engine.add_failure_mode("s", fm).unwrap();
    assert!(has_issue(&state, "fm", IssueType::MissingScore));
    // Unscored => clarify signal.
    assert!(state
        .signals
        .iter()
        .any(|sig| sig.failure_mode_id == "fm" && sig.kind == SignalKind::Clarify));
}

#[test]
fn unmitigated_high_detected() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    // High/High => High criticality, no mitigation.
    let state = engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    assert!(has_issue(&state, "fm", IssueType::UnmitigatedHigh));
    // notify (high criticality) + remediate (unmitigated) signals.
    assert!(state
        .signals
        .iter()
        .any(|sig| sig.failure_mode_id == "fm" && sig.kind == SignalKind::Notify));
    assert!(state
        .signals
        .iter()
        .any(|sig| sig.failure_mode_id == "fm" && sig.kind == SignalKind::Remediate));
}

#[test]
fn unmitigated_medium_detected() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    // Medium/Medium => Medium criticality, no mitigation.
    let state = engine
        .add_failure_mode("s", failure_mode("s", "fm", Medium, Medium))
        .unwrap();
    assert!(has_issue(&state, "fm", IssueType::UnmitigatedMedium));
    assert!(!has_issue(&state, "fm", IssueType::UnmitigatedHigh));
}

#[test]
fn weak_mitigation_order_detected_for_fail_fast_only() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    // Only a fail_fast mitigation (even one reducing residual to Low) => weak order.
    let state = engine
        .add_mitigation("s", mitigation("s", "m", "fm", FailFast, Low, Low))
        .unwrap();
    assert!(has_issue(&state, "fm", IssueType::WeakMitigationOrder));
}

#[test]
fn weak_mitigation_order_not_raised_when_prevention_present() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    engine
        .add_mitigation("s", mitigation("s", "m1", "fm", Prevention, Low, Low))
        .unwrap();
    let state = engine
        .add_mitigation("s", mitigation("s", "m2", "fm", FailFast, Low, Low))
        .unwrap();
    assert!(!has_issue(&state, "fm", IssueType::WeakMitigationOrder));
}

#[test]
fn residual_still_high_detected_for_under_mitigation() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    // A detection mitigation that only brings residual to High/High (still High).
    let state = engine
        .add_mitigation("s", mitigation("s", "m", "fm", Detection, High, High))
        .unwrap();
    assert!(has_issue(&state, "fm", IssueType::ResidualStillHigh));
    // remediate signal present.
    assert!(state
        .signals
        .iter()
        .any(|sig| sig.failure_mode_id == "fm" && sig.kind == SignalKind::Remediate));
}

#[test]
fn residual_uses_best_mitigation_across_several() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    // weak (still High) then strong (to Low): best residual wins.
    engine
        .add_mitigation("s", mitigation("s", "m1", "fm", Detection, High, High))
        .unwrap();
    let state = engine
        .add_mitigation("s", mitigation("s", "m2", "fm", Prevention, Low, Low))
        .unwrap();
    let fmv = state
        .failure_modes
        .iter()
        .find(|f| f.failure_mode.id == "fm")
        .unwrap();
    assert_eq!(fmv.residual_criticality, Some(fmeca::Level::Low));
    assert!(!has_issue(&state, "fm", IssueType::ResidualStillHigh));
}

#[test]
fn clean_failure_mode_has_no_issues() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm", High, High))
        .unwrap();
    let state = engine
        .add_mitigation("s", mitigation("s", "m", "fm", Prevention, Low, Low))
        .unwrap();
    let fm_issues: Vec<_> = state
        .issues
        .iter()
        .filter(|i| i.failure_mode_id == "fm")
        .collect();
    assert!(
        fm_issues.is_empty(),
        "expected no issues, got {fm_issues:?}"
    );
}
