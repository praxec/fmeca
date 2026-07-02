//! Stateless one-shot FMECA (`analyze`): hand the kernel an ENTIRE analysis in
//! one call, get back the ENTIRE computed report.
//!
//! This is a pure function — NO session, NO persistence, NO event log on disk —
//! just `(input) → (computed report)`, deterministic + idempotent. It reuses the
//! EXACT SAME kernel compute as the session path so it CANNOT diverge: it builds
//! an in-memory `Vec<Event>` (`SessionOpened` + `FailureModeAdded` +
//! `MitigationAdded`) and runs the canonical [`crate::projection::project`] fold.
//! The criticality matrix, residual rule, standing, `response_class`, issue
//! detection, and the readiness gate are therefore the very same code that
//! `state.get` / `append` use — there is no second copy of the scoring logic.
//!
//! Observation ids are validated against the SELECTED strategy's catalog exactly
//! as the [`crate::engine::Engine`] does at write time; an unknown id yields
//! [`FmecaError::InvalidObservation`].
//!
//! See the compute-parity test in `tests/analyze.rs`
//! (`analyze_agrees_with_session_path`) which builds the same FMECA via the
//! `Engine` session and asserts byte-identical criticality + readiness.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::event::Event;
use crate::matrix::MatrixStrategy;
use crate::model::{
    Criticality, Domain, EntityRef, EvidenceRef, FailureMode, FailureModeStanding, Issue, Level,
    Mitigation, MitigationKind,
};
use crate::projection::{self, FailureModeView};
use crate::response::{ResponseClass, Scope};
use crate::scoring::{self, Axis};

/// A self-contained mitigation in an [`AnalyzeInput`] (mirrors [`Mitigation`]
/// without the redundant `session_id` / `failure_mode_id` — those are implied by
/// the enclosing failure mode in the batch). The residual S/P is supplied as
/// OBSERVATIONS, never a `Level` directly — exactly like the
/// session path; the kernel derives the residual level via the SAME scoring map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzeMitigation {
    pub id: String,
    pub kind: MitigationKind,
    pub description: String,
    /// Residual-severity observations (scoring-catalog ids on the `severity`
    /// axis for the selected strategy). Empty => no derived residual severity.
    #[serde(default)]
    pub residual_severity_observations: Vec<String>,
    /// Residual-probability observations (scoring-catalog ids on the
    /// `probability` axis for the selected strategy). Empty => no derived
    /// residual probability.
    #[serde(default)]
    pub residual_probability_observations: Vec<String>,
}

/// A self-contained failure mode + its mitigations for the stateless
/// [`analyze`] path. Mirrors a [`FailureMode`] but inlines the mitigations and
/// drops the session-scoped fields (no session exists). Observations are
/// validated against the SELECTED strategy's catalog, identically to the session
/// path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzeInput {
    pub id: String,
    pub component: EntityRef,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    pub domain: Domain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub severity_observations: Vec<String>,
    #[serde(default)]
    pub probability_observations: Vec<String>,
    #[serde(default)]
    pub mitigations: Vec<AnalyzeMitigation>,
}

/// One computed failure-mode line in an [`AnalyzeReport`]. The same computed
/// values the session projection produces (criticality, residual, standing,
/// response_class), plus the per-FM issues detected for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzeFailureMode {
    pub id: String,
    pub criticality: Option<Criticality>,
    pub residual_criticality: Option<Criticality>,
    pub standing: Option<FailureModeStanding>,
    pub response_class: Option<ResponseClass>,
    pub issues: Vec<Issue>,
}

/// The full computed report of a stateless [`analyze`] call. Deterministic +
/// idempotent: the same input always yields the same report. `ready` / `blockers`
/// are computed by the SAME readiness gate as the session path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzeReport {
    pub matrix_strategy: MatrixStrategy,
    pub failure_modes: Vec<AnalyzeFailureMode>,
    /// Failure-mode ids ordered by criticality desc, then residual criticality
    /// desc (highest risk first). Ties keep insertion order (stable sort).
    pub risk_ranking: Vec<String>,
    /// Every issue across all failure modes, in projection order.
    pub issues: Vec<Issue>,
    pub ready: bool,
    pub blockers: Vec<String>,
}

/// The synthetic session id used for the in-memory event stream. Never persisted;
/// it only satisfies the event-shape requirement that each event names a session.
const ANALYZE_SESSION_ID: &str = "__analyze__";

/// Run a stateless, one-shot FMECA over a batch of failure modes (the `analyze`
/// batch tool). Pure function: builds an in-memory event stream and
/// runs the canonical projection — NO store, NO persistence — so it cannot
/// diverge from the session path.
///
/// Observation ids are validated against `strategy`'s catalog (same rule as the
/// engine); an unknown id returns [`FmecaError::InvalidObservation`].
pub fn analyze(strategy: MatrixStrategy, failure_modes: &[AnalyzeInput]) -> Result<AnalyzeReport> {
    // Validate observations up front, exactly as the engine does at write time,
    // so an unknown id fails fast with the stable INVALID_OBSERVATION prefix
    // before any compute. (The projection itself tolerates unknown ids by
    // yielding None — that is the replay-safety contract — so we validate here
    // to surface the caller bug rather than silently treating it as unscored.)
    for fm in failure_modes {
        scoring::derive_ordinal(strategy, Axis::Severity, &fm.severity_observations)?;
        scoring::derive_ordinal(strategy, Axis::Probability, &fm.probability_observations)?;
        // Note: the residual is observation-derived too — validate each
        // mitigation's residual observations against the SELECTED strategy, same
        // rule as the unmitigated axes and the session path.
        for mit in &fm.mitigations {
            scoring::derive_ordinal(
                strategy,
                Axis::Severity,
                &mit.residual_severity_observations,
            )?;
            scoring::derive_ordinal(
                strategy,
                Axis::Probability,
                &mit.residual_probability_observations,
            )?;
        }
    }

    let events = to_events(strategy, failure_modes);
    let state = projection::project(&events);

    let report_fms: Vec<AnalyzeFailureMode> = state
        .failure_modes
        .iter()
        .map(|fmv| AnalyzeFailureMode {
            id: fmv.failure_mode.id.clone(),
            criticality: fmv.criticality,
            residual_criticality: fmv.residual_criticality,
            standing: fmv.standing,
            response_class: fmv.response_class,
            issues: issues_for(&state.issues, &fmv.failure_mode.id),
        })
        .collect();

    let risk_ranking = rank_by_risk(&state.failure_modes);

    Ok(AnalyzeReport {
        matrix_strategy: state.matrix_strategy,
        failure_modes: report_fms,
        risk_ranking,
        issues: state.issues,
        ready: state.readiness.ready,
        blockers: state.readiness.blockers,
    })
}

/// Build the in-memory event stream that the canonical projection folds. This is
/// the SAME shape the engine appends, so `project` produces an identical
/// `FmecaState` — guaranteeing zero divergence.
fn to_events(strategy: MatrixStrategy, failure_modes: &[AnalyzeInput]) -> Vec<Event> {
    let mut events = Vec::with_capacity(1 + failure_modes.len() * 2);
    events.push(Event::SessionOpened {
        session_id: ANALYZE_SESSION_ID.to_string(),
        matrix_strategy: strategy,
    });
    for input in failure_modes {
        events.push(Event::FailureModeAdded {
            failure_mode: Box::new(to_failure_mode(input)),
        });
        for mit in &input.mitigations {
            events.push(Event::MitigationAdded {
                mitigation: Box::new(to_mitigation(&input.id, mit)),
            });
        }
    }
    events
}

fn to_failure_mode(input: &AnalyzeInput) -> FailureMode {
    FailureMode {
        id: input.id.clone(),
        session_id: ANALYZE_SESSION_ID.to_string(),
        component: input.component.clone(),
        description: input.description.clone(),
        cause: input.cause.clone(),
        effect: input.effect.clone(),
        severity_observations: input.severity_observations.clone(),
        probability_observations: input.probability_observations.clone(),
        domain: input.domain,
        scope: input.scope,
        // The stateless path has no conversational turn; provenance defaults to
        // the failure-mode id so the projection still carries a stable source.
        source: EvidenceRef::new(input.id.clone()),
    }
}

fn to_mitigation(fm_id: &str, mit: &AnalyzeMitigation) -> Mitigation {
    Mitigation {
        id: mit.id.clone(),
        session_id: ANALYZE_SESSION_ID.to_string(),
        failure_mode_id: fm_id.to_string(),
        kind: mit.kind,
        description: mit.description.clone(),
        residual_severity_observations: mit.residual_severity_observations.clone(),
        residual_probability_observations: mit.residual_probability_observations.clone(),
        source: EvidenceRef::new(mit.id.clone()),
    }
}

/// The issues raised against a specific failure mode, in projection order.
fn issues_for(issues: &[Issue], fm_id: &str) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.failure_mode_id == fm_id)
        .cloned()
        .collect()
}

/// Order failure-mode ids by criticality desc, then residual criticality desc
/// (highest risk first). Unscored (`None`) sorts last on each key. Stable: ties
/// keep insertion order.
fn rank_by_risk(failure_modes: &[FailureModeView]) -> Vec<String> {
    let mut ranked: Vec<&FailureModeView> = failure_modes.iter().collect();
    ranked.sort_by(|a, b| {
        risk_key(b.criticality)
            .cmp(&risk_key(a.criticality))
            .then_with(|| risk_key(b.residual_criticality).cmp(&risk_key(a.residual_criticality)))
    });
    ranked
        .into_iter()
        .map(|fmv| fmv.failure_mode.id.clone())
        .collect()
}

/// Sort key for a criticality bucket: higher is worse, unscored sorts lowest.
fn risk_key(c: Option<Criticality>) -> u8 {
    match c {
        Some(Level::High) => 3,
        Some(Level::Medium) => 2,
        Some(Level::Low) => 1,
        None => 0,
    }
}
