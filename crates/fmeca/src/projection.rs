//! The [`FmecaState`] fold: replay the append-only log into a live
//! snapshot with **computed** criticality, residual risk, and standing.
//!
//! Nothing computed is ever stored — criticality, residual, and standing are
//! pure functions of the events. The fold happens in passes:
//!  1. Collect failure modes (applying re-scores), grouped mitigations, and the
//!     registry of component ids.
//!  2. Compute each failure mode's raw criticality, best residual criticality,
//!     and discrete [`FailureModeStanding`].
//!  3. Run the detectors ([`crate::detect`]) to produce issues + signals, then
//!     the readiness gate ([`crate::readiness`]).
//!
//! Residual rule: the **best (lowest-criticality)** residual among a
//! failure mode's mitigations; with no mitigation the residual is the raw S/P
//! criticality.
//!
//! Standing rules:
//!  - `acceptable`     — residual criticality is `Low`.
//!  - `under_mitigated`— has ≥1 mitigation but residual criticality is High/Medium.
//!  - `unmitigated`    — has no mitigation and raw criticality is High/Medium.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::detect::{self, DetectionOutput};
use crate::event::Event;
use crate::matrix::MatrixStrategy;
use crate::model::{Criticality, FailureMode, FailureModeStanding, Issue, Level, Mitigation};
use crate::readiness::ReadinessReport;
use crate::response::{self, ResponseClass};
use crate::scoring::{self, Axis};
use crate::signal::Signal;

/// The session registry: the component ids seen so far, so callers
/// reuse stable identity instead of re-declaring it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub component_ids: Vec<String>,
}

/// A failure mode paired with its computed criticality, residual, standing, and
/// the mitigations applied to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureModeView {
    #[serde(flatten)]
    pub failure_mode: FailureMode,
    /// Raw criticality from the unmitigated S/P, or `None` if not yet scored.
    pub criticality: Option<Criticality>,
    /// Best (lowest) residual criticality across mitigations, or the raw
    /// criticality when no mitigation exists. `None` if not yet scored *and*
    /// unmitigated.
    pub residual_criticality: Option<Criticality>,
    /// Computed standing. `None` until the failure mode is scored.
    pub standing: Option<FailureModeStanding>,
    /// Deterministically-derived remediation magnitude.
    /// `None` until the failure mode is scored.
    pub response_class: Option<ResponseClass>,
    /// Code-derived unmitigated severity, as a level on the SESSION'S strategy
    /// scale (note: from the `severity_observations`, never supplied
    /// by the model). `None` if no severity observation was given. Under the
    /// default 3×3 strategy this is one of low/medium/high; under NASA 5×5 it is
    /// one of the 5 levels.
    pub derived_severity: Option<crate::matrix::StrategyLevel>,
    /// Code-derived unmitigated probability on the session's strategy scale
    ///. `None` if no probability observation was given.
    pub derived_probability: Option<crate::matrix::StrategyLevel>,
    /// Mitigations applied to this failure mode, in append order.
    pub mitigations: Vec<Mitigation>,
}

impl FailureModeView {
    /// True when the failure mode has cause, effect, and a derived
    /// severity+probability — i.e. it can be analyzed (`clarify`).
    /// Severity/probability now come from observations.
    pub fn is_scored(&self) -> bool {
        self.failure_mode.cause.is_some()
            && self.failure_mode.effect.is_some()
            && self.derived_severity.is_some()
            && self.derived_probability.is_some()
    }
}

/// Derive the unmitigated (severity, probability) ordinals from a failure mode's
/// observations under the SESSION'S strategy. Observations are
/// validated at write time against the session's strategy, so an unexpected
/// unknown id here yields `None` rather than panicking — replay must never panic
/// on persisted data.
pub(crate) fn derived_ordinals(
    strategy: MatrixStrategy,
    fm: &FailureMode,
) -> (Option<u8>, Option<u8>) {
    let severity = scoring::derive_ordinal(strategy, Axis::Severity, &fm.severity_observations)
        .ok()
        .flatten();
    let probability =
        scoring::derive_ordinal(strategy, Axis::Probability, &fm.probability_observations)
            .ok()
            .flatten();
    (severity, probability)
}

/// Resolve an ordinal to the strategy's [`StrategyLevel`] (ordinal + label) for
/// surfacing in the projection. An out-of-scale ordinal (impossible on validated
/// data) yields `None` rather than panicking.
fn strategy_level(strategy: MatrixStrategy, ordinal: u8) -> Option<crate::matrix::StrategyLevel> {
    strategy.scale().into_iter().find(|l| l.ordinal == ordinal)
}

/// The full computed projection of a session (`FmecaState`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FmecaState {
    pub session_id: String,
    /// The criticality-matrix strategy selected when this session was opened
    ///. Fixed for the life of the session; recorded in `SessionOpened`.
    pub matrix_strategy: MatrixStrategy,
    /// The active strategy's ordered scale (ordinal + label), so the caller knows
    /// how many levels its observations resolve to.
    pub matrix_scale: Vec<crate::matrix::StrategyLevel>,
    /// Live failure modes with their computed criticality/residual/standing, in
    /// insertion order.
    pub failure_modes: Vec<FailureModeView>,
    pub issues: Vec<Issue>,
    pub signals: Vec<Signal>,
    pub registry: Registry,
    pub readiness: ReadinessReport,
    /// The ACTIVE strategy's fixed scoring-criteria catalog,
    /// surfaced so the caller knows the exact observation vocabulary for
    /// `severity_observations` / `probability_observations` under this session's
    /// strategy.
    pub scoring_catalog: Vec<crate::scoring::ScoreCriterion>,
}

/// Intermediate raw fold of the log, before criticality/standing are computed.
#[derive(Debug, Default)]
pub(crate) struct RawFold {
    pub session_id: String,
    /// The strategy recorded on `SessionOpened`; default 3×3.
    pub matrix_strategy: MatrixStrategy,
    /// Failure modes in insertion order (re-scores applied in place).
    pub failure_modes: Vec<FailureMode>,
    /// Mitigations in append order.
    pub mitigations: Vec<Mitigation>,
}

/// Fold a replayed event stream into a [`RawFold`] (pass 1). A `Rescored` event
/// mutates the targeted failure mode's unmitigated S/P in place.
pub(crate) fn raw_fold(events: &[Event]) -> RawFold {
    let mut fold = RawFold::default();
    for event in events {
        match event {
            Event::SessionOpened {
                session_id,
                matrix_strategy,
            } => {
                fold.session_id = session_id.clone();
                fold.matrix_strategy = *matrix_strategy;
            }
            Event::FailureModeAdded { failure_mode } => {
                fold.failure_modes.push((**failure_mode).clone());
            }
            Event::MitigationAdded { mitigation } => {
                fold.mitigations.push((**mitigation).clone());
            }
            Event::Rescored { rescore } => {
                if let Some(fm) = fold
                    .failure_modes
                    .iter_mut()
                    .find(|fm| fm.id == rescore.failure_mode_id)
                {
                    // Note: a rescore replaces the OBSERVATIONS; the
                    // level is re-derived during projection.
                    fm.severity_observations = rescore.severity_observations.clone();
                    fm.probability_observations = rescore.probability_observations.clone();
                }
            }
        }
    }
    fold
}

/// Build the registry from the raw fold: every component id seen,
/// sorted & deduped for stable output.
fn build_registry(fold: &RawFold) -> Registry {
    let ids: BTreeSet<String> = fold
        .failure_modes
        .iter()
        .map(|fm| fm.component.id.clone())
        .collect();
    Registry {
        component_ids: ids.into_iter().collect(),
    }
}

/// Mitigations applied to a given failure mode, in append order.
pub(crate) fn mitigations_for<'a>(fold: &'a RawFold, fm_id: &str) -> Vec<&'a Mitigation> {
    fold.mitigations
        .iter()
        .filter(|m| m.failure_mode_id == fm_id)
        .collect()
}

/// Raw criticality of a failure mode from its code-derived unmitigated S/P
/// ordinals, collapsed through the SESSION'S [`MatrixStrategy`] to the public
/// {Low|Medium|High} bucket. Takes derived ordinals (from
/// observations), not levels supplied by the caller. The richer 5×5 input
/// collapses to L/M/H here, leaving everything downstream unchanged.
pub(crate) fn raw_criticality(
    strategy: MatrixStrategy,
    severity: Option<u8>,
    probability: Option<u8>,
) -> Option<Criticality> {
    match (severity, probability) {
        (Some(s), Some(p)) => Some(strategy.criticality(s, p)),
        _ => None,
    }
}

/// One mitigation's residual criticality. The residual S/P is
/// derived from the mitigation's OBSERVATIONS via the SAME
/// [`scoring::derive_ordinal`] used for the unmitigated axes (MAX-combine within
/// the session's strategy), then collapsed by the SAME
/// [`MatrixStrategy::criticality`]. There is NO second scoring path. `None` when
/// either residual axis is unobserved (the mitigation contributes no residual).
///
/// Observations are validated at write time against the session's strategy, so an
/// unexpected unknown id here yields `None` rather than panicking — replay must
/// never panic on persisted data.
pub(crate) fn mitigation_residual(
    strategy: MatrixStrategy,
    mitigation: &Mitigation,
) -> Option<Criticality> {
    let sev = scoring::derive_ordinal(
        strategy,
        Axis::Severity,
        &mitigation.residual_severity_observations,
    )
    .ok()
    .flatten()?;
    let prob = scoring::derive_ordinal(
        strategy,
        Axis::Probability,
        &mitigation.residual_probability_observations,
    )
    .ok()
    .flatten()?;
    Some(strategy.criticality(sev, prob))
}

/// Best (lowest-criticality) residual across a failure mode's mitigations
///. With no mitigation, the residual is the raw S/P criticality. Each
/// mitigation's residual is derived from its OBSERVATIONS via
/// [`mitigation_residual`] — the SAME scoring + matrix used for the unmitigated
/// axes. A mitigation whose residual axes are unobserved contributes nothing.
pub(crate) fn residual_criticality(
    strategy: MatrixStrategy,
    raw: Option<Criticality>,
    mitigations: &[&Mitigation],
) -> Option<Criticality> {
    if mitigations.is_empty() {
        return raw;
    }
    mitigations
        .iter()
        .filter_map(|m| mitigation_residual(strategy, m))
        .min_by_key(|c| c.rank())
        // Mitigations exist but none yields a derived residual: fall back to the
        // raw criticality so an unscored-residual mitigation never silently
        // improves standing.
        .or(raw)
}

/// Discrete standing of a failure mode. `None` until it is scored.
fn standing_of(
    mitigations: &[&Mitigation],
    residual: Option<Criticality>,
) -> Option<FailureModeStanding> {
    let residual = residual?;
    if residual == Level::Low {
        return Some(FailureModeStanding::Acceptable);
    }
    // Residual is High/Medium.
    if mitigations.is_empty() {
        // Unscored failure modes have residual == None above, so reaching here
        // with no mitigation means it is scored and unmitigated.
        Some(FailureModeStanding::Unmitigated)
    } else {
        Some(FailureModeStanding::UnderMitigated)
    }
}

/// Build the complete [`FmecaState`] from a replayed event stream. This is the
/// single deterministic entry point used by both `state.get` and `append`.
pub fn project(events: &[Event]) -> FmecaState {
    let fold = raw_fold(events);
    let registry = build_registry(&fold);
    let strategy = fold.matrix_strategy;

    let failure_modes: Vec<FailureModeView> = fold
        .failure_modes
        .iter()
        .map(|fm| {
            let mits = mitigations_for(&fold, &fm.id);
            let (sev_ordinal, prob_ordinal) = derived_ordinals(strategy, fm);
            let raw = raw_criticality(strategy, sev_ordinal, prob_ordinal);
            let residual = residual_criticality(strategy, raw, &mits);
            let standing = standing_of(&mits, residual);
            let response_class = response::response_class(raw, fm.domain, fm.scope);
            FailureModeView {
                failure_mode: fm.clone(),
                criticality: raw,
                residual_criticality: residual,
                standing,
                response_class,
                derived_severity: sev_ordinal.and_then(|o| strategy_level(strategy, o)),
                derived_probability: prob_ordinal.and_then(|o| strategy_level(strategy, o)),
                mitigations: mits.into_iter().cloned().collect(),
            }
        })
        .collect();

    let detection: DetectionOutput = detect::detect(&failure_modes);
    let readiness = crate::readiness::assess(&failure_modes, &detection.issues);
    let signals = crate::signal::compute_signals(&failure_modes, &detection);

    FmecaState {
        session_id: fold.session_id.clone(),
        matrix_strategy: strategy,
        matrix_scale: strategy.scale(),
        failure_modes,
        issues: detection.issues,
        signals,
        registry,
        readiness,
        scoring_catalog: scoring::catalog_for(strategy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level::{High, Low, Medium};
    use crate::model::{EvidenceRef, MitigationKind};

    /// Build a mitigation carrying explicit residual OBSERVATIONS.
    fn mit(residual_sev: Vec<&str>, residual_prob: Vec<&str>) -> Mitigation {
        Mitigation {
            id: "m".to_string(),
            session_id: "s".to_string(),
            failure_mode_id: "fm".to_string(),
            kind: MitigationKind::Prevention,
            description: "m".to_string(),
            residual_severity_observations: residual_sev.into_iter().map(String::from).collect(),
            residual_probability_observations: residual_prob
                .into_iter()
                .map(String::from)
                .collect(),
            source: EvidenceRef::new("t"),
        }
    }

    // --- residual derive TABLE (mirrors the FailureMode-axis tests) ----------

    #[test]
    fn residual_observations_derive_residual_criticality_3x3() {
        let q = MatrixStrategy::Qualitative3x3;
        // cosmetic→low sev, rare_edge_case→low prob ⇒ Low residual.
        assert_eq!(
            mitigation_residual(q, &mit(vec!["cosmetic"], vec!["rare_edge_case"])),
            Some(Low)
        );
        // data_loss→high sev, happens_in_normal_use→high prob ⇒ High residual.
        assert_eq!(
            mitigation_residual(q, &mit(vec!["data_loss"], vec!["happens_in_normal_use"])),
            Some(High)
        );
        // user_facing_degradation→medium sev, occasional→medium prob ⇒ Medium.
        assert_eq!(
            mitigation_residual(q, &mit(vec!["user_facing_degradation"], vec!["occasional"])),
            Some(Medium)
        );
    }

    #[test]
    fn residual_observations_max_combine() {
        let q = MatrixStrategy::Qualitative3x3;
        // severity: cosmetic(low) + data_loss(high) → high; probability low ⇒
        // criticality(High, Low) = Medium.
        assert_eq!(
            mitigation_residual(
                q,
                &mit(vec!["cosmetic", "data_loss"], vec!["rare_edge_case"])
            ),
            Some(Medium)
        );
    }

    #[test]
    fn residual_empty_axis_yields_none() {
        let q = MatrixStrategy::Qualitative3x3;
        // empty severity residual ⇒ no derived residual for this mitigation.
        assert_eq!(
            mitigation_residual(q, &mit(vec![], vec!["rare_edge_case"])),
            None
        );
        assert_eq!(mitigation_residual(q, &mit(vec![], vec![])), None);
    }

    #[test]
    fn residual_unknown_id_does_not_panic_yields_none_in_projection() {
        // Replay-safety: an unknown id (rejected at write time) must not panic
        // during the fold; it degrades to None.
        let q = MatrixStrategy::Qualitative3x3;
        assert_eq!(
            mitigation_residual(q, &mit(vec!["not_a_real_id"], vec!["rare_edge_case"])),
            None
        );
    }

    #[test]
    fn residual_cross_strategy_id_yields_none_in_projection() {
        // A NASA id under the 3×3 strategy is unknown ⇒ None (write-time
        // validation is what surfaces the INVALID_OBSERVATION error).
        let q = MatrixStrategy::Qualitative3x3;
        assert_eq!(
            mitigation_residual(
                q,
                &mit(vec!["loss_of_life_or_mission"], vec!["near_certain"])
            ),
            None
        );
    }

    #[test]
    fn residual_nasa_5x5_uses_same_derivation_and_matrix() {
        let n = MatrixStrategy::Nasa8004_5x5;
        // (5,5) ⇒ High; (1,1) ⇒ Low; (3,3) ⇒ Medium — same matrix as the
        // unmitigated axes.
        assert_eq!(
            mitigation_residual(
                n,
                &mit(vec!["loss_of_life_or_mission"], vec!["near_certain"])
            ),
            Some(High)
        );
        assert_eq!(
            mitigation_residual(n, &mit(vec!["negligible_impact"], vec!["improbable"])),
            Some(Low)
        );
        assert_eq!(
            mitigation_residual(n, &mit(vec!["degraded_capability"], vec!["occasional"])),
            Some(Medium)
        );
    }

    // --- best-residual selection + unscored-residual fallback ----------------

    #[test]
    fn best_residual_wins_across_mitigations() {
        let q = MatrixStrategy::Qualitative3x3;
        let weak = mit(vec!["data_loss"], vec!["happens_in_normal_use"]); // High
        let strong = mit(vec!["cosmetic"], vec!["rare_edge_case"]); // Low
        let mits = vec![&weak, &strong];
        assert_eq!(
            residual_criticality(q, Some(High), &mits),
            Some(Low),
            "best (lowest) residual wins"
        );
    }

    #[test]
    fn unscored_residual_mitigation_falls_back_to_raw() {
        // A mitigation with an unobserved residual must NOT silently improve
        // standing: residual falls back to the raw criticality.
        let q = MatrixStrategy::Qualitative3x3;
        let unscored = mit(vec![], vec![]);
        let mits = vec![&unscored];
        assert_eq!(residual_criticality(q, Some(High), &mits), Some(High));
    }
}
