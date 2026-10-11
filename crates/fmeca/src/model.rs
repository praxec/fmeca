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
    /// The higher-level failure mode this one refines (spec §6.2). Validated
    /// on append (the parent session and failure mode must exist), never on
    /// replay, so logs stay portable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
    /// External anchors: `cpm:<plan>/<deliverable>`, `path:<file>`,
    /// `issue:<url>`, `pr:<url>`, `commit:<sha>`. Free-form; not validated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    /// Local effect (spec §6.6.1). The existing `effect` is the END effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_effect: Option<String>,
    /// Next-higher-level effect (spec §6.6.1). With a `parent`, this should
    /// describe how the failure shows up as the parent's cause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_effect: Option<String>,
    /// Single-point failure (spec §6.6.4): with High raw criticality and not
    /// excluded, the failure mode is a critical item.
    #[serde(default, skip_serializing_if = "is_false")]
    pub single_point: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The level of an analysis (spec §6.2). One analysis per level; analyses are
/// linked between levels via [`SessionMeta::parent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisLevel {
    Concept,
    Conops,
    TechnicalPlan,
    ProjectPlan,
    Implementation,
    Verification,
}

/// Analysis metadata recorded on `SessionOpened`. Every field is optional, so
/// old logs replay with `SessionMeta::default()`. Fixed once a session is
/// opened, like the matrix strategy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<AnalysisLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_paths: Vec<String>,
    /// The parent session id (e.g. a technical plan under its conops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// A link from a failure mode to the failure mode it refines, possibly in
/// another session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRef {
    pub session_id: String,
    pub failure_mode_id: String,
}

/// Why a piece of evidence counts (spec §6.6.3; NASA retention-rationale
/// categories). `kind` says where the evidence is; `basis` says why it counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceBasis {
    Design,
    Test,
    Inspection,
    FailureHistory,
    OperationalUse,
}

/// A failure mode's disposition (spec §6.6.2). Computed as the last
/// `FailureModeDispositioned` event, or `Open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Under analysis; counts toward every gate.
    #[default]
    Open,
    /// Residual risk retained on purpose (needs rationale + approver). Keeps
    /// its standing and residual, but blocks neither gate.
    Accepted,
    /// Not credible (needs rationale + non-note evidence). Left out of
    /// `risk.next`, the buckets and both gates; kept in state, never deleted.
    Excluded,
}

/// A disposition change for a failure mode (the `FailureModeDispositioned`
/// event payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispositionChange {
    pub session_id: String,
    pub failure_mode_id: String,
    pub disposition: Disposition,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approver: Option<String>,
    #[serde(default)]
    pub evidence: Vec<ResolutionEvidence>,
    /// RFC 3339 UTC, stamped by the server; the kernel never reads a clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// A mitigation's resolution status (spec §6.3). Every mitigation starts
/// `Planned`; any transition is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MitigationStatus {
    #[default]
    Planned,
    Implemented,
    Verified,
    /// No longer counts toward residual risk or standing.
    Abandoned,
}

/// Where a piece of resolution evidence lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Commit,
    PullRequest,
    Test,
    File,
    Url,
    Note,
}

/// One piece of evidence for a status or disposition change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionEvidence {
    pub kind: EvidenceKind,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<EvidenceBasis>,
}

impl ResolutionEvidence {
    /// True when this item is something other than a free-text note.
    pub fn is_substantive(&self) -> bool {
        self.kind != EvidenceKind::Note
    }
}

/// A mitigation status change (the `MitigationStatusChanged` event payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MitigationStatusChange {
    pub session_id: String,
    pub mitigation_id: String,
    pub status: MitigationStatus,
    #[serde(default)]
    pub evidence: Vec<ResolutionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// RFC 3339 UTC, stamped by the server; the kernel never reads a clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
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
