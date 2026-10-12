//! The policy document model (spec §3).
//!
//! Every struct rejects unknown keys (`deny_unknown_fields`), and every map is a
//! [`BTreeMap`], so a parsed [`Policy`] serialises to one canonical JSON form —
//! the input to [`policy_hash`](crate::policy_hash).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The only policy schema version this crate understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Evidence confidence, `C0` (declared only) to `C4` (operational use or
/// failure history). Ordered: `C0 < C1 < … < C4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Confidence {
    /// Declared only: no evidence, or only `note` items.
    C0,
    /// Analysed: basis `design`, or kind `file`/`url` without a basis.
    C1,
    /// Inspected: basis `inspection`, or kind `pull_request`/`commit`.
    C2,
    /// Tested: basis `test` or kind `test`.
    C3,
    /// Proven: basis `operational_use` or `failure_history`.
    C4,
}

/// A suggested disposition (the NASA NPR 8000.4C vocabulary).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionKind {
    /// Retain the risk on purpose.
    Accept,
    /// Reduce the risk with an action.
    Mitigate,
    /// Monitor the risk without acting now.
    Watch,
    /// Gather information before deciding.
    Research,
    /// The risk no longer needs tracking.
    Close,
    /// Hand the decision to a higher authority.
    Elevate,
}

/// Action precedence (MIL-STD-882E §4.3.4, adapted). Ordered: `Eliminate` is
/// the most preferred, `Procedure` the least.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precedence {
    /// Remove the hazard by design.
    Eliminate,
    /// Stop the failure from happening.
    Prevent,
    /// Notice the failure when it happens.
    Detect,
    /// Limit the failure's effects.
    Contain,
    /// Rely on a procedure or warning.
    Procedure,
}

/// A risk band: a named rung of the policy's risk ladder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Band {
    /// Unique band id, referenced by matrices, sign-off and actions.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Unique rank; higher is worse.
    pub rank: u8,
    /// Dispositions suggested for a failure mode in this band.
    pub dispositions: Vec<DispositionKind>,
    /// Leaving a failure mode in this band without action needs a rationale.
    #[serde(default)]
    pub no_action_needs_rationale: bool,
    /// A residual counts in this band only at or above this confidence;
    /// below it, the inherent band applies.
    pub min_confidence: Option<Confidence>,
    /// Suggested role to accept a risk left in this band.
    pub acceptance: Option<String>,
    /// Roles that must concur with an acceptance.
    #[serde(default)]
    pub concurrence: Vec<String>,
    /// How often an accepted risk in this band is reviewed, as `"<n>d"`.
    pub review_interval: Option<String>,
}

/// A policy matrix mapping two axes to band ids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    /// The built-in layout this matrix re-colours: `"qualitative3x3"`,
    /// `"nasa8004_5x5"` (S×P; rows severity ascending, columns probability
    /// ascending) or `"standard-bands"` (the `confidence` matrix; rows
    /// `standard` band rank ascending, columns `C4`…`C0`).
    pub derived_from: String,
    /// Band ids, row by row.
    pub cells: Vec<Vec<String>>,
}

fn yes() -> bool {
    true
}

/// The sign-off rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signoff {
    /// Effective bands ranked above this one fail sign-off unless accepted.
    pub max_residual_band: String,
    /// An accepted risk does not fail sign-off (default `true`).
    #[serde(default = "yes")]
    pub accepted_counts_as_pass: bool,
    /// …unless its review is overdue (default `false`).
    #[serde(default)]
    pub accepted_requires_review_current: bool,
    /// When true (the default), the analysis's kernel readiness blockers
    /// (CLARIFY, weak mitigation order, EMPTY) are added to the policy's
    /// sign-off reasons.
    #[serde(default = "yes")]
    pub include_kernel_gate: bool,
}

/// Conditions on a failure mode; every condition present must hold. An empty
/// set always holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    /// The effective band ranks at least this band.
    pub band_at_least: Option<String>,
    /// The effective band is one of these.
    pub band_in: Option<Vec<String>>,
    /// The analysis level is one of these.
    pub level_in: Option<Vec<fmeca::AnalysisLevel>>,
    /// The failure mode's domain is one of these.
    pub domain_in: Option<Vec<fmeca::Domain>>,
    /// The derived response class is one of these.
    pub response_class_in: Option<Vec<fmeca::ResponseClass>>,
    /// The failure mode is (or is not) a single point of failure.
    pub single_point: Option<bool>,
    /// The residual confidence is below this.
    pub confidence_below: Option<Confidence>,
    /// The residual confidence is at least this.
    pub confidence_at_least: Option<Confidence>,
    /// The failure mode's disposition is one of these.
    pub disposition_in: Option<Vec<fmeca::Disposition>>,
    /// The effective band did (or did not) fall back to the inherent band.
    pub fallback: Option<bool>,
    /// The policy's class is one of these (for tailoring).
    pub class_in: Option<Vec<String>>,
    /// None of the active mitigations is of any of these kinds.
    pub mitigation_kinds_missing: Option<Vec<fmeca::MitigationKind>>,
}

/// A suggested action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    /// Action id.
    pub id: String,
    /// What to do.
    pub text: String,
    /// Free tags.
    pub tags: Vec<String>,
    /// Where the action sits in the precedence order.
    pub precedence: Precedence,
    /// When the action is suggested (default: always).
    #[serde(default)]
    pub applies_when: Conditions,
    /// When a suggested action is required rather than advised.
    pub required_when: Option<Conditions>,
}

/// A risk policy document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Policy name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Free class label (presets use `prototype`, `standard`,
    /// `business-critical`, `safety-critical`).
    pub class: String,
    /// Schema version; must be [`SCHEMA_VERSION`].
    pub schema: u32,
    /// The band ladder.
    pub bands: Vec<Band>,
    /// Matrices by key: `"qualitative3x3"`, `"nasa8004_5x5"`, optionally
    /// `"confidence"`.
    pub matrices: BTreeMap<String, Matrix>,
    /// The sign-off rule.
    pub signoff: Signoff,
    /// Policy-specific actions.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Also suggest the crate's built-in actions.
    #[serde(default)]
    pub include_builtin_actions: bool,
}
