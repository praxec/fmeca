//! Append → replay roundtrip: replaying the same event log
//! yields an identical [`FmecaState`], and a fresh engine over the same on-disk
//! store reproduces the projection exactly (restart-survival).

mod common;

use std::sync::Arc;

use common::{failure_mode, mitigation, rescore, temp_engine};
use fmeca::Level::{High, Low, Medium};
use fmeca::MitigationKind::Prevention;
use fmeca::{Engine, FilesystemStore};

#[test]
fn replay_is_deterministic_and_idempotent() {
    let (engine, _d) = temp_engine();
    let s = "rt";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm1", High, High))
        .unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm2", Medium, Medium))
        .unwrap();
    engine
        .add_mitigation(s, mitigation(s, "m1", "fm1", Prevention, Low, Low))
        .unwrap();
    engine.rescore(s, rescore(s, "fm2", Low, Low)).unwrap();

    let a = engine.state(s).unwrap();
    let b = engine.state(s).unwrap();
    assert_eq!(a, b, "two reads of the same log must be identical");
}

#[test]
fn fresh_engine_over_same_store_reproduces_state() {
    let dir = tempfile::tempdir().unwrap();
    let s = "persist";

    let state_before = {
        let store = FilesystemStore::new(dir.path()).unwrap();
        let engine = Engine::new(Arc::new(store));
        engine.open_session(s).unwrap();
        engine
            .add_failure_mode(s, failure_mode(s, "fm1", High, High))
            .unwrap();
        engine
            .add_mitigation(s, mitigation(s, "m1", "fm1", Prevention, Low, Low))
            .unwrap();
        engine.rescore(s, rescore(s, "fm1", Medium, Low)).unwrap()
    };

    // New engine, same dir — restart survival is pure replay.
    let store = FilesystemStore::new(dir.path()).unwrap();
    let engine = Engine::new(Arc::new(store));
    let state_after = engine.state(s).unwrap();

    assert_eq!(
        state_before, state_after,
        "projection must survive a restart unchanged"
    );
}

#[test]
fn rescore_changes_raw_but_residual_follows_mitigation() {
    let (engine, _d) = temp_engine();
    let s = "resc";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm", High, High))
        .unwrap();
    let before = engine.state(s).unwrap();
    let fm_before = &before.failure_modes[0];
    assert_eq!(fm_before.criticality, Some(High));

    // Rescore down to Low/Low.
    let after = engine.rescore(s, rescore(s, "fm", Low, Low)).unwrap();
    let fm_after = &after.failure_modes[0];
    assert_eq!(fm_after.criticality, Some(Low));
    assert_eq!(fm_after.residual_criticality, Some(Low));
}
