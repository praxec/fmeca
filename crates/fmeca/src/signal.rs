//! First-class output signals: returned on **every** `append` and
//! **every** read so the caller can never forget to check.
//!
//! Three kinds:
//!  - `notify`    — a High/Medium criticality failure mode exists.
//!  - `clarify`   — a failure mode is missing cause/effect/score (can't be
//!    analyzed).
//!  - `remediate` — an unmitigated/under-mitigated High/Medium failure mode,
//!    carrying the prevent→detect→fail-fast mitigation-order guidance.

use serde::{Deserialize, Serialize};

use crate::detect::DetectionOutput;
use crate::model::{IssueType, Level};
use crate::projection::FailureModeView;

/// The kind of a [`Signal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Notify,
    Clarify,
    Remediate,
}

/// A single actionable signal for the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    pub kind: SignalKind,
    /// Stable code for programmatic dispatch, e.g. `high_criticality`,
    /// `missing_analysis`, `unmitigated_risk`.
    pub code: String,
    /// Human-facing message.
    pub message: String,
    /// The failure mode this signal concerns.
    pub failure_mode_id: String,
    /// Mitigation-order guidance, present on `remediate` signals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
}

/// Compute the signal set from the failure-mode views and detector output
///. Order is deterministic: notify (High/Medium criticality), then
/// remediate (unmitigated/under-mitigated), then clarify (gaps), each in
/// failure-mode insertion order.
pub(crate) fn compute_signals(
    failure_modes: &[FailureModeView],
    detection: &DetectionOutput,
) -> Vec<Signal> {
    let mut signals = Vec::new();

    // notify: any High/Medium *raw* criticality failure mode exists.
    for fmv in failure_modes {
        if matches!(fmv.criticality, Some(Level::High) | Some(Level::Medium)) {
            let level = match fmv.criticality {
                Some(Level::High) => "High",
                _ => "Medium",
            };
            signals.push(Signal {
                kind: SignalKind::Notify,
                code: "high_criticality".to_string(),
                message: format!(
                    "Failure mode '{}' is {level} criticality.",
                    fmv.failure_mode.id
                ),
                failure_mode_id: fmv.failure_mode.id.clone(),
                suggested_action: None,
            });
        }
    }

    // remediate: unmitigated/under-mitigated High/Medium, with order guidance.
    for issue in &detection.issues {
        match issue.r#type {
            IssueType::UnmitigatedHigh
            | IssueType::UnmitigatedMedium
            | IssueType::ResidualStillHigh => {
                signals.push(Signal {
                    kind: SignalKind::Remediate,
                    code: "unmitigated_risk".to_string(),
                    message: issue.explanation.clone(),
                    failure_mode_id: issue.failure_mode_id.clone(),
                    suggested_action: Some(issue.suggested_action.clone()),
                });
            }
            IssueType::WeakMitigationOrder => {
                signals.push(Signal {
                    kind: SignalKind::Remediate,
                    code: "weak_mitigation_order".to_string(),
                    message: issue.explanation.clone(),
                    failure_mode_id: issue.failure_mode_id.clone(),
                    suggested_action: Some(issue.suggested_action.clone()),
                });
            }
            _ => {}
        }
    }

    // clarify: each completeness gap needs a question to the human.
    for issue in &detection.issues {
        match issue.r#type {
            IssueType::MissingCause | IssueType::MissingEffect | IssueType::MissingScore => {
                signals.push(Signal {
                    kind: SignalKind::Clarify,
                    code: "missing_analysis".to_string(),
                    message: issue.explanation.clone(),
                    failure_mode_id: issue.failure_mode_id.clone(),
                    suggested_action: Some(issue.suggested_action.clone()),
                });
            }
            _ => {}
        }
    }

    signals
}

/// The highest-leverage failure mode to address next (`risk.next`): the
/// highest-criticality **unmitigated** failure mode. Returns `None` when nothing
/// is unmitigated High/Medium.
pub fn next_risk(failure_modes: &[FailureModeView]) -> Option<&FailureModeView> {
    failure_modes
        .iter()
        .filter(|fmv| {
            fmv.mitigations.is_empty()
                && matches!(fmv.criticality, Some(Level::High) | Some(Level::Medium))
        })
        .max_by_key(|fmv| fmv.criticality.map(|c| c.rank()).unwrap_or(0))
}
