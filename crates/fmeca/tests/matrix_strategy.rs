//! Swappable matrix-strategy seam: the strategy is SELECTED at
//! `session.open`, recorded in the event log, and drives the active scoring
//! catalog + the (severity,probability)→Criticality collapse. The criticality
//! OUTPUT stays {Low|Medium|High} for every strategy so the rest of the pipeline
//! (readiness, response_class, signals) is unchanged.

mod common;

use std::sync::Arc;

use common::{failure_mode, failure_mode_obs, temp_engine};
use fmeca::Level::{High, Low, Medium};
use fmeca::{Criticality, Engine, FilesystemStore, MatrixStrategy};

// --- selection: default + explicit + persisted-through-replay ----------------

#[test]
fn default_strategy_is_qualitative_3x3() {
    let (engine, _d) = temp_engine();
    let st = engine.open_session("s").unwrap();
    assert_eq!(st.matrix_strategy, MatrixStrategy::Qualitative3x3);
    assert_eq!(st.matrix_scale.len(), 3);
}

#[test]
fn explicit_strategy_is_recorded_in_state() {
    let (engine, _d) = temp_engine();
    let st = engine
        .open_session_with("s", MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    assert_eq!(st.matrix_strategy, MatrixStrategy::Nasa8004_5x5);
    assert_eq!(st.matrix_scale.len(), 5);
    // The active scoring catalog is the NASA catalog (5-level labels present).
    assert!(
        st.scoring_catalog
            .iter()
            .any(|c| c.id == "loss_of_life_or_mission" && c.level_ordinal == 5)
    );
    // ...and none of the 3×3 ids leak in.
    assert!(!st.scoring_catalog.iter().any(|c| c.id == "data_loss"));
}

#[test]
fn strategy_is_fixed_once_opened() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with("s", MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    // Re-opening with a different strategy is ignored — the session's choice stands.
    let st = engine
        .open_session_with("s", MatrixStrategy::Qualitative3x3)
        .unwrap();
    assert_eq!(st.matrix_strategy, MatrixStrategy::Nasa8004_5x5);
}

#[test]
fn strategy_persists_through_replay_on_a_fresh_engine() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = FilesystemStore::new(dir.path()).unwrap();
        let engine = Engine::new(Arc::new(store));
        engine
            .open_session_with("persist", MatrixStrategy::Nasa8004_5x5)
            .unwrap();
        engine
            .add_failure_mode(
                "persist",
                failure_mode_obs(
                    "persist",
                    "fm1",
                    vec!["loss_of_life_or_mission".into()],
                    vec!["near_certain".into()],
                ),
            )
            .unwrap();
    }
    // Fresh engine over the same store: the strategy and its derivation survive.
    let store = FilesystemStore::new(dir.path()).unwrap();
    let engine = Engine::new(Arc::new(store));
    let st = engine.state("persist").unwrap();
    assert_eq!(st.matrix_strategy, MatrixStrategy::Nasa8004_5x5);
    let fmv = &st.failure_modes[0];
    // catastrophic(5) × near_certain(5) ⇒ High.
    assert_eq!(fmv.criticality, Some(High));
    assert_eq!(fmv.derived_severity.as_ref().unwrap().ordinal, 5);
    assert_eq!(fmv.derived_severity.as_ref().unwrap().label, "catastrophic");
    assert_eq!(fmv.derived_probability.as_ref().unwrap().ordinal, 5);
}

// --- NASA 5×5 criticality table: all 25 cells golden, THROUGH the engine -----

/// Authoritative locked NASA table `[consequence-1][likelihood-1]`.
const NASA_TABLE: [[Criticality; 5]; 5] = [
    [Low, Low, Low, Medium, Medium],
    [Low, Low, Medium, Medium, High],
    [Low, Medium, Medium, High, High],
    [Medium, Medium, High, High, High],
    [Medium, High, High, High, High],
];

/// A severity observation id that derives to the given NASA ordinal.
fn nasa_sev(ordinal: u8) -> String {
    match ordinal {
        5 => "loss_of_life_or_mission",
        4 => "major_system_damage",
        3 => "degraded_capability",
        2 => "minor_impact",
        _ => "negligible_impact",
    }
    .to_string()
}

/// A probability observation id that derives to the given NASA ordinal.
fn nasa_prob(ordinal: u8) -> String {
    match ordinal {
        5 => "near_certain",
        4 => "probable",
        3 => "occasional",
        2 => "remote",
        _ => "improbable",
    }
    .to_string()
}

#[test]
fn nasa_5x5_projection_computes_every_one_of_the_25_cells() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with("nasa", MatrixStrategy::Nasa8004_5x5)
        .unwrap();

    let mut seen = 0;
    for c in 1..=5u8 {
        for l in 1..=5u8 {
            let id = format!("fm_{c}_{l}");
            let st = engine
                .add_failure_mode(
                    "nasa",
                    failure_mode_obs("nasa", &id, vec![nasa_sev(c)], vec![nasa_prob(l)]),
                )
                .unwrap();
            let fmv = st
                .failure_modes
                .iter()
                .find(|f| f.failure_mode.id == id)
                .expect("failure mode present");
            let want = NASA_TABLE[(c - 1) as usize][(l - 1) as usize];
            assert_eq!(
                fmv.criticality,
                Some(want),
                "NASA cell (C={c}, L={l}) projected criticality must be {want:?}"
            );
            // Unmitigated ⇒ residual == raw.
            assert_eq!(fmv.residual_criticality, Some(want));
            seen += 1;
        }
    }
    assert_eq!(seen, 25, "all 25 NASA cells must be exercised");
}

// --- 5-level catalog derive: MAX-combine, empty, unknown ---------------------

#[test]
fn nasa_max_combine_picks_worst_observation() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with("mc", MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    // marginal(2) + critical(4) severity ⇒ derived ordinal 4; near_certain(5)
    // probability ⇒ critical×near-certain ⇒ High.
    let st = engine
        .add_failure_mode(
            "mc",
            failure_mode_obs(
                "mc",
                "fm",
                vec!["minor_impact".into(), "major_system_damage".into()],
                vec!["near_certain".into()],
            ),
        )
        .unwrap();
    let fmv = &st.failure_modes[0];
    assert_eq!(fmv.derived_severity.as_ref().unwrap().ordinal, 4);
    assert_eq!(fmv.criticality, Some(High));
}

#[test]
fn nasa_empty_observations_leave_axis_unscored() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with("e", MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    let st = engine
        .add_failure_mode(
            "e",
            failure_mode_obs("e", "fm", vec![], vec!["near_certain".into()]),
        )
        .unwrap();
    let fmv = &st.failure_modes[0];
    assert!(fmv.derived_severity.is_none());
    assert_eq!(fmv.criticality, None);
    // missing_score blocks readiness, as for 3×3.
    assert!(!st.readiness.ready);
}

#[test]
fn nasa_unknown_observation_is_rejected_at_write() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with("u", MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    let err = engine
        .add_failure_mode(
            "u",
            failure_mode_obs(
                "u",
                "fm",
                vec!["not_a_real_id".into()],
                vec!["near_certain".into()],
            ),
        )
        .unwrap_err();
    assert!(err.to_string().starts_with("INVALID_OBSERVATION:"));
}

// --- cross-strategy isolation: a NASA id is unknown under 3×3 (and vice versa)

#[test]
fn nasa_observation_id_is_invalid_under_3x3_session() {
    let (engine, _d) = temp_engine();
    // Default (3×3) session.
    engine.open_session("q").unwrap();
    let err = engine
        .add_failure_mode(
            "q",
            failure_mode_obs(
                "q",
                "fm",
                vec!["loss_of_life_or_mission".into()],
                vec!["happens_in_normal_use".into()],
            ),
        )
        .unwrap_err();
    assert!(err.to_string().starts_with("INVALID_OBSERVATION:"));
}

#[test]
fn three_by_three_observation_id_is_invalid_under_nasa_session() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with("n", MatrixStrategy::Nasa8004_5x5)
        .unwrap();
    let err = engine
        .add_failure_mode(
            "n",
            failure_mode_obs(
                "n",
                "fm",
                vec!["data_loss".into()],
                vec!["near_certain".into()],
            ),
        )
        .unwrap_err();
    assert!(err.to_string().starts_with("INVALID_OBSERVATION:"));
}

// --- back-compat: the default 3×3 session behaves EXACTLY as before -----------

#[test]
fn default_session_still_uses_3x3_observation_vocabulary() {
    let (engine, _d) = temp_engine();
    engine.open_session("bc").unwrap();
    // The classic helper (data_loss / happens_in_normal_use ⇒ High/High ⇒ High).
    let st = engine
        .add_failure_mode("bc", failure_mode("bc", "fm1", High, High))
        .unwrap();
    let fmv = &st.failure_modes[0];
    assert_eq!(fmv.criticality, Some(High));
    assert_eq!(fmv.derived_severity.as_ref().unwrap().label, "high");
    assert_eq!(st.matrix_strategy, MatrixStrategy::Qualitative3x3);
    // Medium/Low path too.
    let st = engine
        .add_failure_mode("bc", failure_mode("bc", "fm2", Medium, Low))
        .unwrap();
    let fmv = st
        .failure_modes
        .iter()
        .find(|f| f.failure_mode.id == "fm2")
        .unwrap();
    assert_eq!(fmv.criticality, Some(Low));
}
