//! Mitigation resolution lifecycle: status, evidence, history, the verified
//! readiness gate, and resolution counts.

mod common;

use common::{evidence, failure_mode, mitigation, status_change, temp_engine};
use fmeca::EvidenceKind::{Commit, Note};
use fmeca::Level::{High, Low};
use fmeca::MitigationKind::Prevention;
use fmeca::MitigationStatus::{Abandoned, Implemented, Planned, Verified};
use fmeca::{Engine, EvidenceBasis, ResolutionCounts};

/// The existing "fully mitigated to Low" fixture: one High/High failure mode with
/// one prevention mitigation whose residual is Low/Low. Declared-ready.
fn mitigated_session() -> (Engine, tempfile::TempDir) {
    let (engine, dir) = temp_engine();
    engine.open_session("s").unwrap();
    engine
        .add_failure_mode("s", failure_mode("s", "fm-1", High, High))
        .unwrap();
    engine
        .add_mitigation("s", mitigation("s", "m-1", "fm-1", Prevention, Low, Low))
        .unwrap();
    (engine, dir)
}

#[test]
fn new_mitigation_is_planned() {
    let (engine, _d) = mitigated_session();
    let state = engine.state("s").unwrap();
    assert_eq!(state.failure_modes[0].mitigations[0].status, Planned);
}

#[test]
fn status_change_updates_status() {
    let (engine, _d) = mitigated_session();
    let state = engine
        .set_mitigation_status("s", status_change("s", "m-1", Implemented, vec![]))
        .unwrap();
    assert_eq!(state.failure_modes[0].mitigations[0].status, Implemented);
}

#[test]
fn history_keeps_every_change() {
    let (engine, _d) = mitigated_session();
    engine
        .set_mitigation_status("s", status_change("s", "m-1", Implemented, vec![]))
        .unwrap();
    let state = engine
        .set_mitigation_status(
            "s",
            status_change("s", "m-1", Verified, vec![evidence(Commit, "abc123")]),
        )
        .unwrap();
    assert_eq!(state.failure_modes[0].mitigations[0].history.len(), 2);
}

#[test]
fn verified_requires_non_note_evidence() {
    let (engine, _d) = mitigated_session();
    let result = engine.set_mitigation_status(
        "s",
        status_change("s", "m-1", Verified, vec![evidence(Note, "trust me")]),
    );
    assert!(result.is_err());
}

#[test]
fn unknown_mitigation_is_rejected() {
    let (engine, _d) = mitigated_session();
    let result =
        engine.set_mitigation_status("s", status_change("s", "m-nope", Implemented, vec![]));
    assert!(result.is_err());
}

#[test]
fn abandoned_mitigation_does_not_reduce_residual() {
    let (engine, _d) = mitigated_session();
    let state = engine
        .set_mitigation_status("s", status_change("s", "m-1", Abandoned, vec![]))
        .unwrap();
    let fmv = &state.failure_modes[0];
    assert_eq!(fmv.residual_criticality, fmv.criticality);
}

#[test]
fn declared_readiness_unchanged_by_planned_status() {
    let (engine, _d) = mitigated_session();
    assert!(engine.readiness("s").unwrap().ready);
}

#[test]
fn verified_gate_blocks_until_mitigation_verified() {
    let (engine, _d) = mitigated_session();
    assert!(!engine.readiness("s").unwrap().verified.ready);
}

#[test]
fn verified_gate_passes_after_verification() {
    let (engine, _d) = mitigated_session();
    engine
        .set_mitigation_status(
            "s",
            status_change("s", "m-1", Verified, vec![evidence(Commit, "abc123")]),
        )
        .unwrap();
    assert!(engine.readiness("s").unwrap().verified.ready);
}

#[test]
fn resolution_counts_by_status() {
    let (engine, _d) = mitigated_session();
    engine
        .add_mitigation("s", mitigation("s", "m-2", "fm-1", Prevention, Low, Low))
        .unwrap();
    engine
        .set_mitigation_status("s", status_change("s", "m-2", Abandoned, vec![]))
        .unwrap();
    assert_eq!(
        engine.readiness("s").unwrap().resolution,
        ResolutionCounts {
            planned: 1,
            implemented: 0,
            verified: 0,
            abandoned: 1,
        }
    );
}

#[test]
fn export_includes_mitigation_status() {
    let (engine, _d) = mitigated_session();
    engine
        .set_mitigation_status("s", status_change("s", "m-1", Implemented, vec![]))
        .unwrap();
    assert_eq!(
        engine.export("s").unwrap().rows[0].mitigations[0].status,
        Implemented
    );
}

#[test]
fn evidence_basis_round_trips() {
    let (engine, _d) = mitigated_session();
    let mut item = evidence(Commit, "abc123");
    item.basis = Some(EvidenceBasis::Test);
    let state = engine
        .set_mitigation_status("s", status_change("s", "m-1", Verified, vec![item]))
        .unwrap();
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        json["failure_modes"][0]["mitigations"][0]["evidence"][0]["basis"],
        "test"
    );
}
