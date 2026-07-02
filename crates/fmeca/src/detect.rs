//! Issue detectors.
//!
//! Pure functions over the computed [`FailureModeView`] set. Each detector
//! raises a typed [`Issue`]; the table test exercises one per [`IssueType`].
//!
//! Detectors:
//!  - `missing_cause` / `missing_effect` / `missing_score` — the failure mode
//!    can't be analyzed yet (drives the `clarify` signal).
//!  - `unmitigated_high` / `unmitigated_medium` — a High/Medium criticality
//!    failure mode with **no** mitigation (drives `remediate`).
//!  - `weak_mitigation_order` — a failure mode mitigated **only** by `fail_fast`
//!    when no `prevention`/`detection` exists (prevention preferred).
//!  - `residual_still_high` — mitigated, but residual criticality is still
//!    High/Medium (under-mitigated; drives `remediate`).

use crate::model::{FailureModeStanding, IssueType, Level, MitigationKind};
use crate::model::{Issue, Mitigation};
use crate::projection::FailureModeView;

/// The output of running every detector over the projection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DetectionOutput {
    pub issues: Vec<Issue>,
}

/// Run all detectors over the failure-mode views. Deterministic: issue
/// order follows failure-mode insertion order, then a fixed per-FM detector
/// order.
pub(crate) fn detect(failure_modes: &[FailureModeView]) -> DetectionOutput {
    let mut issues = Vec::new();
    for fmv in failure_modes {
        detect_one(fmv, &mut issues);
    }
    DetectionOutput { issues }
}

fn detect_one(fmv: &FailureModeView, issues: &mut Vec<Issue>) {
    let fm = &fmv.failure_mode;

    // --- completeness gaps (drive `clarify`) -------------------------------
    if fm.cause.is_none() {
        issues.push(Issue {
            id: format!("issue:{}:missing_cause", fm.id),
            r#type: IssueType::MissingCause,
            failure_mode_id: fm.id.clone(),
            severity: Level::Medium,
            explanation: format!("Failure mode '{}' has no cause.", fm.id),
            suggested_action: "Provide the cause of this failure mode so it can be analyzed."
                .to_string(),
        });
    }
    if fm.effect.is_none() {
        issues.push(Issue {
            id: format!("issue:{}:missing_effect", fm.id),
            r#type: IssueType::MissingEffect,
            failure_mode_id: fm.id.clone(),
            severity: Level::Medium,
            explanation: format!("Failure mode '{}' has no effect.", fm.id),
            suggested_action: "Provide the effect of this failure mode so it can be analyzed."
                .to_string(),
        });
    }
    if fmv.derived_severity.is_none() || fmv.derived_probability.is_none() {
        issues.push(Issue {
            id: format!("issue:{}:missing_score", fm.id),
            r#type: IssueType::MissingScore,
            failure_mode_id: fm.id.clone(),
            severity: Level::Medium,
            explanation: format!(
                "Failure mode '{}' is missing a severity and/or probability observation.",
                fm.id
            ),
            suggested_action:
                "Supply severity/probability OBSERVATIONS (ids from the scoring catalog); \
                 the engine derives the level."
                    .to_string(),
        });
    }

    // --- unmitigated High/Medium (drive `remediate`) -----------------------
    if fmv.mitigations.is_empty() {
        match fmv.criticality {
            Some(Level::High) => issues.push(Issue {
                id: format!("issue:{}:unmitigated_high", fm.id),
                r#type: IssueType::UnmitigatedHigh,
                failure_mode_id: fm.id.clone(),
                severity: Level::High,
                explanation: format!(
                    "Failure mode '{}' is High criticality with no mitigation.",
                    fm.id
                ),
                suggested_action: mitigation_order_guidance(),
            }),
            Some(Level::Medium) => issues.push(Issue {
                id: format!("issue:{}:unmitigated_medium", fm.id),
                r#type: IssueType::UnmitigatedMedium,
                failure_mode_id: fm.id.clone(),
                severity: Level::Medium,
                explanation: format!(
                    "Failure mode '{}' is Medium criticality with no mitigation.",
                    fm.id
                ),
                suggested_action: mitigation_order_guidance(),
            }),
            _ => {}
        }
    } else {
        // --- weak mitigation order -------------------------------
        if only_fail_fast(&fmv.mitigations) {
            issues.push(Issue {
                id: format!("issue:{}:weak_mitigation_order", fm.id),
                r#type: IssueType::WeakMitigationOrder,
                failure_mode_id: fm.id.clone(),
                severity: Level::Medium,
                explanation: format!(
                    "Failure mode '{}' is mitigated only by fail_fast; prevention is preferred.",
                    fm.id
                ),
                suggested_action:
                    "Prefer a prevention or detection mitigation before relying on fail_fast."
                        .to_string(),
            });
        }

        // --- residual still High/Medium (under-mitigated) ------------------
        if fmv.standing == Some(FailureModeStanding::UnderMitigated) {
            issues.push(Issue {
                id: format!("issue:{}:residual_still_high", fm.id),
                r#type: IssueType::ResidualStillHigh,
                failure_mode_id: fm.id.clone(),
                severity: residual_severity(fmv),
                explanation: format!(
                    "Failure mode '{}' still has {} residual criticality after mitigation.",
                    fm.id,
                    residual_label(fmv),
                ),
                suggested_action: "Strengthen mitigation until residual criticality reaches Low."
                    .to_string(),
            });
        }
    }
}

/// True when every mitigation is `fail_fast` (and there is at least one).
fn only_fail_fast(mitigations: &[Mitigation]) -> bool {
    !mitigations.is_empty()
        && mitigations
            .iter()
            .all(|m| m.kind == MitigationKind::FailFast)
}

fn residual_severity(fmv: &FailureModeView) -> Level {
    fmv.residual_criticality.unwrap_or(Level::Medium)
}

fn residual_label(fmv: &FailureModeView) -> &'static str {
    match fmv.residual_criticality {
        Some(Level::High) => "High",
        Some(Level::Medium) => "Medium",
        Some(Level::Low) => "Low",
        None => "unknown",
    }
}

fn mitigation_order_guidance() -> String {
    "Add a mitigation following the prevent→detect→fail-fast discipline (prevention first)."
        .to_string()
}
