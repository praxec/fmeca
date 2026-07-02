//! Shared test builders for the fmeca golden/table tests.
//!
//! Each integration-test binary compiles this module separately, so helpers used
//! by only some binaries are dead code in others.
#![allow(dead_code)]

use std::sync::Arc;

use fmeca::{
    Domain, Engine, EntityRef, EvidenceRef, FailureMode, FilesystemStore, Level, Mitigation,
    MitigationKind, Rescore,
};

/// A failure mode carrying explicit severity/probability observation ids: used
/// for strategy-specific catalogs (e.g. NASA 5×5) where the level is not one of
/// the 3×3 Low/Medium/High.
pub fn failure_mode_obs(
    session: &str,
    id: &str,
    severity_observations: Vec<String>,
    probability_observations: Vec<String>,
) -> FailureMode {
    FailureMode {
        id: id.to_string(),
        session_id: session.to_string(),
        component: EntityRef::new("comp:svc"),
        description: format!("failure mode {id}"),
        cause: Some("a cause".to_string()),
        effect: Some("an effect".to_string()),
        severity_observations,
        probability_observations,
        domain: Domain::Runtime,
        scope: None,
        source: EvidenceRef::new("turn-1"),
    }
}

/// An engine backed by a fresh temp dir. The `TempDir` is returned so the test
/// keeps it alive for the duration of the test.
pub fn temp_engine() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FilesystemStore::new(dir.path()).expect("store");
    (Engine::new(Arc::new(store)), dir)
}

/// A representative scoring-catalog observation id on the severity axis that
/// derives to the requested [`Level`] (note: tests supply observations,
/// not levels — the kernel derives the level).
pub fn severity_obs(level: Level) -> String {
    match level {
        Level::High => "data_loss",
        Level::Medium => "user_facing_degradation",
        Level::Low => "cosmetic",
    }
    .to_string()
}

/// A representative scoring-catalog observation id on the probability axis that
/// derives to the requested [`Level`].
pub fn probability_obs(level: Level) -> String {
    match level {
        Level::High => "happens_in_normal_use",
        Level::Medium => "occasional",
        Level::Low => "rare_edge_case",
    }
    .to_string()
}

/// A fully-scored failure mode with the given identity and S/P — built from the
/// scoring observations that derive to those levels.
pub fn failure_mode(session: &str, id: &str, severity: Level, probability: Level) -> FailureMode {
    FailureMode {
        id: id.to_string(),
        session_id: session.to_string(),
        component: EntityRef::new("comp:svc"),
        description: format!("failure mode {id}"),
        cause: Some("a cause".to_string()),
        effect: Some("an effect".to_string()),
        severity_observations: vec![severity_obs(severity)],
        probability_observations: vec![probability_obs(probability)],
        domain: Domain::Runtime,
        scope: None,
        source: EvidenceRef::new("turn-1"),
    }
}

/// A mitigation against `fm_id` with the given kind and residual S/P. The
/// residual is supplied as OBSERVATIONS: the helper maps each
/// requested [`Level`] to a representative 3×3 residual observation id so the
/// table-test intent is unchanged while the kernel derives the residual level.
pub fn mitigation(
    session: &str,
    id: &str,
    fm_id: &str,
    kind: MitigationKind,
    residual_severity: Level,
    residual_probability: Level,
) -> Mitigation {
    Mitigation {
        id: id.to_string(),
        session_id: session.to_string(),
        failure_mode_id: fm_id.to_string(),
        kind,
        description: format!("mitigation {id}"),
        residual_severity_observations: vec![severity_obs(residual_severity)],
        residual_probability_observations: vec![probability_obs(residual_probability)],
        source: EvidenceRef::new("turn-2"),
    }
}

/// A rescore of `fm_id` — built from observations deriving to the given levels.
pub fn rescore(session: &str, fm_id: &str, severity: Level, probability: Level) -> Rescore {
    Rescore {
        failure_mode_id: fm_id.to_string(),
        session_id: session.to_string(),
        severity_observations: vec![severity_obs(severity)],
        probability_observations: vec![probability_obs(probability)],
        source: EvidenceRef::new("turn-3"),
    }
}
