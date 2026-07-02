//! Residual-observation tests: the mitigation
//! RESIDUAL S/P is observation-derived, exactly like the unmitigated axes — the
//! caller (an LLM) never supplies a residual `Level`. These tests drive the
//! SESSION path through the `Engine` so the write-time validation + the
//! projection's residual derivation are both exercised, mirroring the
//! FailureMode-axis tests.

mod common;

use fmeca::Level::{High, Low};
use fmeca::MitigationKind::Prevention;
use fmeca::{Domain, EntityRef, EvidenceRef, FmecaError, MatrixStrategy, Mitigation};

use common::{failure_mode, failure_mode_obs, temp_engine};

/// A mitigation carrying explicit residual OBSERVATION ids (not a level).
fn mit_obs(
    session: &str,
    id: &str,
    fm_id: &str,
    residual_sev: Vec<&str>,
    residual_prob: Vec<&str>,
) -> Mitigation {
    Mitigation {
        id: id.to_string(),
        session_id: session.to_string(),
        failure_mode_id: fm_id.to_string(),
        kind: Prevention,
        description: format!("mitigation {id}"),
        residual_severity_observations: residual_sev.into_iter().map(String::from).collect(),
        residual_probability_observations: residual_prob.into_iter().map(String::from).collect(),
        source: EvidenceRef::new("turn-2"),
    }
}

#[test]
fn residual_is_derived_from_observations_through_the_session() {
    let (engine, _d) = temp_engine();
    let s = "resid";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm", High, High))
        .unwrap();
    // Residual observations cosmetic(low) + rare_edge_case(low) ⇒ Low residual.
    let state = engine
        .add_mitigation(
            s,
            mit_obs(s, "m", "fm", vec!["cosmetic"], vec!["rare_edge_case"]),
        )
        .unwrap();
    let fmv = &state.failure_modes[0];
    assert_eq!(fmv.criticality, Some(High), "raw criticality unchanged");
    assert_eq!(
        fmv.residual_criticality,
        Some(Low),
        "residual derived from observations"
    );
}

#[test]
fn mitigation_dropping_residual_to_low_flips_ready() {
    // The headline downstream guarantee: residual→Low must still flip ready and
    // standing exactly as before, now that the residual is observation-derived.
    let (engine, _d) = temp_engine();
    let s = "ready";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm", High, High))
        .unwrap();
    let before = engine.readiness(s).unwrap();
    assert!(!before.ready, "unmitigated High is not ready");

    let state = engine
        .add_mitigation(
            s,
            mit_obs(s, "m", "fm", vec!["cosmetic"], vec!["rare_edge_case"]),
        )
        .unwrap();
    assert!(
        state.readiness.ready,
        "residual→Low flips ready; blockers: {:?}",
        state.readiness.blockers
    );
    assert_eq!(
        state.failure_modes[0].standing,
        Some(fmeca::FailureModeStanding::Acceptable)
    );
    assert_eq!(state.readiness.by_criticality.low, 1);
}

#[test]
fn unknown_residual_observation_is_invalid_observation() {
    let (engine, _d) = temp_engine();
    let s = "bad";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm", High, High))
        .unwrap();
    let err = engine
        .add_mitigation(
            s,
            mit_obs(s, "m", "fm", vec!["not_a_real_id"], vec!["rare_edge_case"]),
        )
        .unwrap_err();
    assert!(matches!(err, FmecaError::InvalidObservation(_)));
    assert!(err.to_string().starts_with("INVALID_OBSERVATION:"));
}

#[test]
fn cross_strategy_residual_observation_is_rejected() {
    // A NASA residual id under the default 3×3 session is unknown.
    let (engine, _d) = temp_engine();
    let s = "xstrat";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm", High, High))
        .unwrap();
    let err = engine
        .add_mitigation(
            s,
            mit_obs(
                s,
                "m",
                "fm",
                vec!["loss_of_life_or_mission"],
                vec!["near_certain"],
            ),
        )
        .unwrap_err();
    assert!(matches!(err, FmecaError::InvalidObservation(_)));
}

#[test]
fn residual_uses_session_strategy_matrix_under_nasa() {
    // Under NASA 5×5, residual observations derive on the 5-level scale and
    // collapse through the SAME matrix: (3,3) ⇒ Medium residual.
    let (engine, _d) = temp_engine();
    let s = "nasa";
    engine
        .open_session_with(s, MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    let mut fm = failure_mode_obs(
        s,
        "fm",
        vec!["loss_of_life_or_mission".to_string()],
        vec!["near_certain".to_string()],
    );
    fm.component = EntityRef::new("comp:svc");
    fm.domain = Domain::Runtime;
    engine.add_failure_mode(s, fm).unwrap();
    let state = engine
        .add_mitigation(
            s,
            mit_obs(
                s,
                "m",
                "fm",
                vec!["degraded_capability"],
                vec!["occasional"],
            ),
        )
        .unwrap();
    assert_eq!(
        state.failure_modes[0].residual_criticality,
        Some(fmeca::Level::Medium)
    );
}

#[test]
fn empty_residual_observations_do_not_improve_standing() {
    // A mitigation with no residual observations must not silently improve the
    // failure mode's standing — residual stays at the raw criticality.
    let (engine, _d) = temp_engine();
    let s = "noresid";
    engine.open_session(s).unwrap();
    engine
        .add_failure_mode(s, failure_mode(s, "fm", High, High))
        .unwrap();
    let state = engine
        .add_mitigation(s, mit_obs(s, "m", "fm", vec![], vec![]))
        .unwrap();
    assert_eq!(state.failure_modes[0].residual_criticality, Some(High));
}
