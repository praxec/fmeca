//! The typed FMECA data model.
//!
//! Identity is **caller-supplied**: the `component` is an
//! [`EntityRef`] with a stable id, and failure-mode / mitigation ids are
//! caller-assigned. Severity and Probability are **qualitative enums**
//! (`Low | Medium | High`); the server computes [`Criticality`], residual risk,
//! and [`FailureModeStanding`] as pure functions — no fuzzy matching, no LLM.
//!
//! ## C2: the model never supplies a score
//!
//! A [`FailureMode`] does NOT carry `severity: Level` / `probability: Level` —
//! that would let the LLM pick the number. Instead the caller supplies
//! OBSERVATIONS ([`crate::scoring`] catalog ids) and the CODE maps them to a
//! [`Level`] via [`crate::scoring::derive_level`]. Downstream — the criticality
//! matrix, residual, standing — is unchanged; only the *input* changed from a
//! `Level` to a set of observation ids.
//!
//! Standing/criticality/residual are **computed**, never stored — they live in
//! the projection ([`crate::projection`]), folded from the append-only log.

use serde::{Deserialize, Serialize};

use crate::response::Scope;

/// A stable, caller-supplied reference to the component under analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl EntityRef {
    /// Build an [`EntityRef`] from a bare id (no label).
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: None,
        }
    }
}

/// Provenance: which conversational turn a fact came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub turn_id: String,
}

impl EvidenceRef {
    /// Build an [`EvidenceRef`] from a turn id.
    pub fn new(turn_id: impl Into<String>) -> Self {
        Self {
            turn_id: turn_id.into(),
        }
    }
}

/// A qualitative risk level. Used for severity, probability, and the
/// computed criticality / residual buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    Medium,
    High,
}

impl Level {
    /// Ordinal rank for comparisons (`Low=0 < Medium=1 < High=2`). The derived
    /// `Ord` already follows declaration order; this is the explicit, tested form.
    pub fn rank(self) -> u8 {
        match self {
            Level::Low => 0,
            Level::Medium => 1,
            Level::High => 2,
        }
    }
}

/// Alias for the criticality bucket — same qualitative scale as [`Level`]
/// (`Criticality = High | Medium | Low`).
pub type Criticality = Level;

/// The FMECA domains from `P_RUN_FMECA`. `Security` is a first-class domain
/// (security failure modes are a distinct concern, not a runtime/architecture
/// sub-case) — added so enumerations that classify a failure mode as `security`
/// are accepted by `analyze` rather than rejected (-32602). It carries no
/// special response_class (defaults to the standard prevention→detection→
/// fail-fast discipline; only `Architecture`/structural maps to ReArchitecture).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Ux,
    Runtime,
    Architecture,
    Delivery,
    Security,
}

/// The mitigation discipline order: prevention is preferred over
/// detection over fail-fast. A failure mode mitigated *only* by `fail_fast`
/// raises a `weak_mitigation_order` issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MitigationKind {
    Prevention,
    Detection,
    FailFast,
}

impl MitigationKind {
    /// Discipline rank: lower is preferred. `prevention=0 < detection=1 <
    /// fail_fast=2`.
    pub fn rank(self) -> u8 {
        match self {
            MitigationKind::Prevention => 0,
            MitigationKind::Detection => 1,
            MitigationKind::FailFast => 2,
        }
    }
}

/// A caller-supplied failure mode. The caller supplies
/// OBSERVATIONS (scoring-catalog ids), never a score directly; the code maps
/// observations → unmitigated severity/probability [`Level`] via
/// [`crate::scoring::derive_level`]. The persisted observations make replay
/// deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureMode {
    pub id: String,
    pub session_id: String,
    pub component: EntityRef,
    pub description: String,
    /// The cause of the failure. `None` => a `missing_cause` issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The effect of the failure. `None` => a `missing_effect` issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    /// Severity observations: scoring-catalog ids on the
    /// `severity` axis. Empty => unscored severity => a `missing_score` issue.
    /// The CODE derives the severity `Level`; the model never supplies it.
    #[serde(default)]
    pub severity_observations: Vec<String>,
    /// Probability observations: scoring-catalog ids on the
    /// `probability` axis. Empty => unscored probability => `missing_score`.
    #[serde(default)]
    pub probability_observations: Vec<String>,
    pub domain: Domain,
    /// Optional breadth of the change surface: sharpens the
    /// computed `response_class` without breaking determinism.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    pub source: EvidenceRef,
}

/// A caller-supplied mitigation against a failure mode.
///
/// The residual S/P the caller expects *after* the mitigation is applied is
/// supplied as OBSERVATIONS — exactly like a [`FailureMode`]'s unmitigated S/P —
/// never as a `Level` directly. This closes the C2 hole on the residual path: an
/// LLM cannot ESTIMATE the residual number; it names which residual observations
/// hold and the CODE derives the residual `Level` via the SAME
/// [`crate::scoring::derive_ordinal`] / [`crate::scoring::derive_level`] used for
/// the unmitigated axes (MAX-combine; empty → None; unknown id →
/// `INVALID_OBSERVATION`; cross-strategy id rejected). The persisted observations
/// make replay deterministic (old logs without the fields default to empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mitigation {
    pub id: String,
    pub session_id: String,
    pub failure_mode_id: String,
    pub kind: MitigationKind,
    pub description: String,
    /// Residual-severity observations (scoring-catalog ids on the `severity`
    /// axis for the active strategy). Empty => this mitigation yields no derived
    /// residual severity (the failure mode's residual then ignores this
    /// mitigation's axis). The CODE derives the residual severity `Level`.
    #[serde(default)]
    pub residual_severity_observations: Vec<String>,
    /// Residual-probability observations (scoring-catalog ids on the
    /// `probability` axis for the active strategy). Empty => no derived residual
    /// probability.
    #[serde(default)]
    pub residual_probability_observations: Vec<String>,
    pub source: EvidenceRef,
}

/// A re-score of an existing failure mode's unmitigated S/P (the `rescore`
/// command). Carries new OBSERVATIONS, not levels — the code re-derives the
/// severity/probability `Level`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rescore {
    pub failure_mode_id: String,
    pub session_id: String,
    /// New severity observations (scoring-catalog ids on the `severity` axis).
    #[serde(default)]
    pub severity_observations: Vec<String>,
    /// New probability observations (scoring-catalog ids on the `probability`
    /// axis).
    #[serde(default)]
    pub probability_observations: Vec<String>,
    pub source: EvidenceRef,
}

/// Computed standing of a failure mode — never stored.
///
/// - `unmitigated`    — High/Medium criticality with no mitigation.
/// - `under_mitigated`— mitigated, but residual criticality is still High/Medium.
/// - `acceptable`     — residual criticality is Low (or raw criticality is Low).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureModeStanding {
    Unmitigated,
    UnderMitigated,
    Acceptable,
}

/// The kinds of issue a detector can raise. Closed set — exhaustively
/// matched, never a hand-maintained string list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    UnmitigatedHigh,
    UnmitigatedMedium,
    MissingCause,
    MissingEffect,
    MissingScore,
    WeakMitigationOrder,
    ResidualStillHigh,
}

/// A detected analysis issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub r#type: IssueType,
    pub failure_mode_id: String,
    /// The qualitative severity of the *issue* (not the failure mode).
    pub severity: Level,
    pub explanation: String,
    pub suggested_action: String,
}

#[cfg(test)]
mod domain_tests {
    use super::Domain;

    #[test]
    fn security_domain_round_trips_as_snake_case() {
        // The agent enumerate step can classify a failure mode as `security`;
        // `analyze` must accept it (previously rejected -32602 → wasted run).
        let d: Domain = serde_json::from_str("\"security\"").expect("security parses");
        assert_eq!(d, Domain::Security);
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"security\"");
    }
}
