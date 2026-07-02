//! Evidence→score mapping: the **code owns the
//! map** from observed evidence to a level WITHIN THE ACTIVE STRATEGY'S SCALE; the
//! caller (an LLM) NEVER supplies a numeric/qualitative score directly.
//!
//! The #1 design principle is that models do the fuzzy *naming* work but never
//! pick the score. So the input model does not take `severity: Level` /
//! `probability: Level`. Instead the caller supplies OBSERVATIONS — ids drawn
//! from the ACTIVE strategy's fixed, code-resident [`catalog_for`] — and the
//! kernel maps them to a strategy-relative ordinal deterministically.
//!
//! The catalog is **data in code** (tunable here, not hardcoded as Rust
//! constants scattered through the engine), but the *mapping policy* lives in
//! code, not the model:
//!  - each [`ScoreCriterion`] pins one observation id, its [`Axis`], the
//!    strategy-relative level (`level_ordinal` + `level` label) that observing it
//!    implies, and a human description;
//!  - [`derive_ordinal`] combines a set of observed ids with a **MAX** rule (the
//!    worst observed evidence wins) within the strategy's scale;
//!  - an empty observation set yields `None` (→ the existing `missing_score`
//!    issue), and an unknown id is an [`FmecaError::InvalidObservation`] error
//!    (stable prefix `INVALID_OBSERVATION`).
//!
//! ## Strategy scoping
//!
//! Observation ids are scoped to a [`MatrixStrategy`]: an id valid under
//! [`MatrixStrategy::Nasa8004_5x5`] is unknown under
//! [`MatrixStrategy::Qualitative3x3`] (→ `INVALID_OBSERVATION`). The legacy
//! [`catalog`] / [`derive_level`] entry points operate on the DEFAULT 3×3
//! strategy and are preserved byte-for-byte for back-compat.

use serde::{Deserialize, Serialize};

use crate::error::{FmecaError, Result};
use crate::matrix::MatrixStrategy;
use crate::model::Level;

/// Which axis a [`ScoreCriterion`] scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Severity,
    Probability,
}

/// One fixed, code-resident scoring criterion: observing `id` on `axis` implies
/// the strategy-relative level (`level_ordinal` within the active strategy's
/// scale, with `level` its label). These are DATA (tunable
/// here), but the map lives in code — never supplied by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreCriterion {
    pub id: String,
    pub axis: Axis,
    /// Strategy-relative ordinal (`1..=N`, higher = worse) on the active
    /// strategy's scale.
    pub level_ordinal: u8,
    /// Human label for `level_ordinal` (for 3×3 this is `low|medium|high`).
    pub level: String,
    pub description: String,
}

impl ScoreCriterion {
    fn new(id: &str, axis: Axis, level_ordinal: u8, level: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            axis,
            level_ordinal,
            level: level.to_string(),
            description: description.to_string(),
        }
    }
}

/// The fixed catalog of scoring criteria for the DEFAULT 3×3 strategy.
/// Preserved verbatim for back-compat (`scoring.catalog` with no selected
/// strategy, and the legacy [`derive_level`] path). Labels are `low|medium|high`.
pub fn catalog() -> Vec<ScoreCriterion> {
    catalog_3x3()
}

/// The catalog for a specific strategy. The active strategy's catalog
/// is surfaced via `scoring.catalog`, `session.open`, and `state.get`.
pub fn catalog_for(strategy: MatrixStrategy) -> Vec<ScoreCriterion> {
    match strategy {
        MatrixStrategy::Qualitative3x3 => catalog_3x3(),
        MatrixStrategy::Nasa8004_5x5 => catalog_nasa_5x5(),
    }
}

/// The historic 3-level catalog (Low=1, Medium=2, High=3). Behaviour is
/// identical to the pre-v3 catalog; the `level` labels match the old `Level`
/// serde strings (`low|medium|high`) so existing callers/tests are unaffected.
fn catalog_3x3() -> Vec<ScoreCriterion> {
    use Axis::{Probability, Severity};
    vec![
        // --- Severity: how bad is the effect if the failure occurs? -----------
        ScoreCriterion::new(
            "data_loss",
            Severity,
            3,
            "high",
            "Permanent loss or corruption of user/system data.",
        ),
        ScoreCriterion::new(
            "security_breach",
            Severity,
            3,
            "high",
            "Confidentiality, integrity, or access-control compromise.",
        ),
        ScoreCriterion::new(
            "service_outage",
            Severity,
            3,
            "high",
            "The component or a dependent becomes unavailable.",
        ),
        ScoreCriterion::new(
            "user_facing_degradation",
            Severity,
            2,
            "medium",
            "Users see degraded behaviour but can still complete their task.",
        ),
        ScoreCriterion::new(
            "recoverable_error",
            Severity,
            2,
            "medium",
            "An error that the system or user can recover from with effort.",
        ),
        ScoreCriterion::new(
            "cosmetic",
            Severity,
            1,
            "low",
            "Cosmetic/non-functional issue with no impact on the task.",
        ),
        // --- Probability: how often is the failure expected to occur? ---------
        ScoreCriterion::new(
            "happens_in_normal_use",
            Probability,
            3,
            "high",
            "Triggered by ordinary, expected usage.",
        ),
        ScoreCriterion::new(
            "known_recurring",
            Probability,
            3,
            "high",
            "Already observed to recur in practice.",
        ),
        ScoreCriterion::new(
            "occasional",
            Probability,
            2,
            "medium",
            "Happens intermittently under common-but-not-constant conditions.",
        ),
        ScoreCriterion::new(
            "load_or_concurrency_dependent",
            Probability,
            2,
            "medium",
            "Surfaces under load, contention, or specific timing.",
        ),
        ScoreCriterion::new(
            "rare_edge_case",
            Probability,
            1,
            "low",
            "Only under an uncommon, narrow edge case.",
        ),
        ScoreCriterion::new(
            "requires_misuse",
            Probability,
            1,
            "low",
            "Only via deliberate misuse or an unsupported configuration.",
        ),
    ]
}

/// The NASA GSFC-HDBK-8004 5-level catalog. Severity (consequence) and
/// probability (likelihood) each map an observation to a `1..=5` ordinal on the
/// strategy's scale (1 = negligible/improbable … 5 = catastrophic/near-certain).
/// MAX-combine as for 3×3. Documented, seeded, code-resident; cells locked.
fn catalog_nasa_5x5() -> Vec<ScoreCriterion> {
    use Axis::{Probability, Severity};
    vec![
        // --- Consequence (severity), 5 = catastrophic … 1 = negligible --------
        ScoreCriterion::new(
            "loss_of_life_or_mission",
            Severity,
            5,
            "catastrophic",
            "Loss of life, total mission loss, or unrecoverable system destruction.",
        ),
        ScoreCriterion::new(
            "permanent_data_or_asset_loss",
            Severity,
            5,
            "catastrophic",
            "Permanent, unrecoverable loss of critical data or a major asset.",
        ),
        ScoreCriterion::new(
            "major_system_damage",
            Severity,
            4,
            "critical",
            "Severe but recoverable damage; major capability lost for an extended period.",
        ),
        ScoreCriterion::new(
            "extended_outage",
            Severity,
            4,
            "critical",
            "Prolonged outage of a primary capability requiring significant recovery.",
        ),
        ScoreCriterion::new(
            "degraded_capability",
            Severity,
            3,
            "moderate",
            "Partial loss of capability; mission/task continues with reduced margin.",
        ),
        ScoreCriterion::new(
            "recoverable_disruption",
            Severity,
            3,
            "moderate",
            "Disruption that is recoverable with planned effort.",
        ),
        ScoreCriterion::new(
            "minor_impact",
            Severity,
            2,
            "marginal",
            "Minor impact; workaround available, little effect on the objective.",
        ),
        ScoreCriterion::new(
            "negligible_impact",
            Severity,
            1,
            "negligible",
            "Negligible effect; no meaningful impact on the objective.",
        ),
        // --- Likelihood (probability), 5 = near-certain … 1 = improbable ------
        ScoreCriterion::new(
            "near_certain",
            Probability,
            5,
            "near_certain",
            "Expected to occur (near-certain) under normal conditions.",
        ),
        ScoreCriterion::new(
            "frequent",
            Probability,
            5,
            "near_certain",
            "Occurs frequently / repeatedly in normal operation.",
        ),
        ScoreCriterion::new(
            "probable",
            Probability,
            4,
            "critical",
            "Will probably occur several times over the life of the system.",
        ),
        ScoreCriterion::new(
            "occasional",
            Probability,
            3,
            "moderate",
            "Likely to occur sometime in the life of the system.",
        ),
        ScoreCriterion::new(
            "remote",
            Probability,
            2,
            "marginal",
            "Unlikely but possible to occur in the life of the system.",
        ),
        ScoreCriterion::new(
            "improbable",
            Probability,
            1,
            "negligible",
            "So unlikely it can be assumed occurrence may not be experienced.",
        ),
    ]
}

/// Look up a single criterion by id on a given axis within a strategy's catalog
/// (cross-axis ids do not match — an id is valid only on the axis it is
/// registered for, and only within its own strategy).
fn find_criterion(strategy: MatrixStrategy, axis: Axis, id: &str) -> Option<ScoreCriterion> {
    catalog_for(strategy)
        .into_iter()
        .find(|c| c.axis == axis && c.id == id)
}

/// Map a set of observation ids on one axis to a strategy-relative ordinal
///. The combine rule is **MAX** — the worst observed evidence
/// wins, within the active strategy's scale.
///
/// - empty set            → `Ok(None)` (→ the existing `missing_score` issue);
/// - all ids known        → `Ok(Some(max ordinal among matches))`;
/// - any unknown id       → `Err(InvalidObservation)` (stable prefix).
///
/// The model never picks the level; it only names which observations hold.
pub fn derive_ordinal(
    strategy: MatrixStrategy,
    axis: Axis,
    observed_ids: &[String],
) -> Result<Option<u8>> {
    let mut best: Option<u8> = None;
    for id in observed_ids {
        let criterion = find_criterion(strategy, axis, id).ok_or_else(|| {
            FmecaError::InvalidObservation(format!(
                "unknown {} observation id '{id}' for strategy '{}'",
                axis_label(axis),
                strategy.id()
            ))
        })?;
        best = Some(match best {
            Some(prev) if prev >= criterion.level_ordinal => prev,
            _ => criterion.level_ordinal,
        });
    }
    Ok(best)
}

/// Legacy DEFAULT-strategy (3×3) helper preserved for back-compat: map a set of
/// observation ids to a qualitative [`Level`] under [`MatrixStrategy::Qualitative3x3`].
/// Behaviour is byte-identical to the pre-v3 `derive_level`.
pub fn derive_level(axis: Axis, observed_ids: &[String]) -> Result<Option<Level>> {
    let ordinal = derive_ordinal(MatrixStrategy::Qualitative3x3, axis, observed_ids)?;
    Ok(ordinal.map(ordinal_to_level_3x3))
}

/// Map a 3×3 ordinal (`1..=3`) to the historic [`Level`] enum.
fn ordinal_to_level_3x3(ordinal: u8) -> Level {
    match ordinal {
        1 => Level::Low,
        2 => Level::Medium,
        _ => Level::High,
    }
}

fn axis_label(axis: Axis) -> &'static str {
    match axis {
        Axis::Severity => "severity",
        Axis::Probability => "probability",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level::{High, Low, Medium};

    #[test]
    fn empty_observations_yield_none() {
        assert_eq!(derive_level(Axis::Severity, &[]).unwrap(), None);
        assert_eq!(derive_level(Axis::Probability, &[]).unwrap(), None);
    }

    #[test]
    fn single_known_observation_maps_to_its_level() {
        assert_eq!(
            derive_level(Axis::Severity, &["data_loss".into()]).unwrap(),
            Some(High)
        );
        assert_eq!(
            derive_level(Axis::Severity, &["user_facing_degradation".into()]).unwrap(),
            Some(Medium)
        );
        assert_eq!(
            derive_level(Axis::Severity, &["cosmetic".into()]).unwrap(),
            Some(Low)
        );
        assert_eq!(
            derive_level(Axis::Probability, &["happens_in_normal_use".into()]).unwrap(),
            Some(High)
        );
        assert_eq!(
            derive_level(Axis::Probability, &["rare_edge_case".into()]).unwrap(),
            Some(Low)
        );
    }

    #[test]
    fn multiple_observations_combine_with_max() {
        // Low + High → High (worst wins).
        assert_eq!(
            derive_level(Axis::Severity, &["cosmetic".into(), "data_loss".into()]).unwrap(),
            Some(High)
        );
        // Low + Medium → Medium.
        assert_eq!(
            derive_level(
                Axis::Probability,
                &["rare_edge_case".into(), "occasional".into()]
            )
            .unwrap(),
            Some(Medium)
        );
        // Order does not matter.
        assert_eq!(
            derive_level(Axis::Severity, &["data_loss".into(), "cosmetic".into()]).unwrap(),
            Some(High)
        );
    }

    #[test]
    fn unknown_observation_is_error() {
        let err = derive_level(Axis::Severity, &["not_a_real_id".into()]).unwrap_err();
        assert!(matches!(err, FmecaError::InvalidObservation(_)));
        assert!(err.to_string().starts_with("INVALID_OBSERVATION:"));
    }

    #[test]
    fn cross_axis_id_does_not_match() {
        // A probability id supplied on the severity axis is unknown there.
        let err = derive_level(Axis::Severity, &["happens_in_normal_use".into()]).unwrap_err();
        assert!(matches!(err, FmecaError::InvalidObservation(_)));
    }

    #[test]
    fn catalog_ids_are_unique_per_axis() {
        for strat in [MatrixStrategy::Qualitative3x3, MatrixStrategy::Nasa8004_5x5] {
            let mut seen = std::collections::BTreeSet::new();
            for c in catalog_for(strat) {
                assert!(
                    seen.insert((c.axis, c.id.clone())),
                    "duplicate catalog id {:?} on {:?} in {:?}",
                    c.id,
                    c.axis,
                    strat
                );
            }
        }
    }

    #[test]
    fn catalog_levels_are_within_strategy_scale() {
        for strat in [MatrixStrategy::Qualitative3x3, MatrixStrategy::Nasa8004_5x5] {
            for c in catalog_for(strat) {
                assert!(
                    strat.is_valid_ordinal(c.level_ordinal),
                    "{:?} criterion {:?} ordinal {} out of scale",
                    strat,
                    c.id,
                    c.level_ordinal
                );
            }
        }
    }

    #[test]
    fn nasa_observations_derive_to_5_level_ordinals() {
        let n = MatrixStrategy::Nasa8004_5x5;
        assert_eq!(
            derive_ordinal(n, Axis::Severity, &["loss_of_life_or_mission".into()]).unwrap(),
            Some(5)
        );
        assert_eq!(
            derive_ordinal(n, Axis::Severity, &["degraded_capability".into()]).unwrap(),
            Some(3)
        );
        assert_eq!(
            derive_ordinal(n, Axis::Severity, &["negligible_impact".into()]).unwrap(),
            Some(1)
        );
        assert_eq!(
            derive_ordinal(n, Axis::Probability, &["near_certain".into()]).unwrap(),
            Some(5)
        );
        assert_eq!(
            derive_ordinal(n, Axis::Probability, &["improbable".into()]).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn nasa_max_combine_and_empty_and_unknown() {
        let n = MatrixStrategy::Nasa8004_5x5;
        // MAX-combine within the 5-level scale: marginal(2) + critical(4) → 4.
        assert_eq!(
            derive_ordinal(
                n,
                Axis::Severity,
                &["minor_impact".into(), "major_system_damage".into()]
            )
            .unwrap(),
            Some(4)
        );
        // empty → None
        assert_eq!(derive_ordinal(n, Axis::Severity, &[]).unwrap(), None);
        // unknown → error
        let err = derive_ordinal(n, Axis::Severity, &["data_loss".into()]).unwrap_err();
        assert!(matches!(err, FmecaError::InvalidObservation(_)));
    }

    #[test]
    fn cross_strategy_ids_are_isolated() {
        // A 3×3 id is unknown under NASA, and a NASA id is unknown under 3×3.
        let n = MatrixStrategy::Nasa8004_5x5;
        let q = MatrixStrategy::Qualitative3x3;
        assert!(derive_ordinal(n, Axis::Severity, &["data_loss".into()]).is_err());
        assert!(derive_ordinal(q, Axis::Severity, &["loss_of_life_or_mission".into()]).is_err());
        // and the legacy 3×3 path rejects a NASA id too
        let err = derive_level(Axis::Severity, &["loss_of_life_or_mission".into()]).unwrap_err();
        assert!(matches!(err, FmecaError::InvalidObservation(_)));
    }
}
