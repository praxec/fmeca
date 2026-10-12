//! The readiness gate: deterministic, computed, honest.
//!
//! `ready == true` **iff** all three conditions hold:
//!  1. Every failure mode has cause + effect + score (no `clarify` / no
//!     `missing_*` issue).
//!  2. No failure mode has a **residual** criticality of High or Medium (all
//!     reduced to Low).
//!  3. No `weak_mitigation_order` issue stands.
//!
//! Dispositions (spec §6.6.2): an `Excluded` failure mode is left out of the
//! buckets and the gates and listed in `excluded`; an `Accepted` one keeps its
//! bucket but raises no blocker and is listed in `accepted_risks`.
//!
//! The `verified` gate (spec §6.3) is the same computation run on the views
//! with every non-verified mitigation removed.
//!
//! The report **shows its work**: residual buckets + every blocker.

use serde::{Deserialize, Serialize};

use crate::model::{Criticality, Disposition, Issue, IssueType, Level, MitigationStatus};
use crate::projection::FailureModeView;

/// Counts of failure modes by their **residual** criticality bucket (also in
/// `report.export`, surfaced in readiness so High/Med is never hidden).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriticalityBuckets {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    /// Failure modes not yet scored (no residual computable).
    pub unscored: usize,
}

impl CriticalityBuckets {
    fn record(&mut self, residual: Option<Criticality>) {
        match residual {
            Some(Level::High) => self.high += 1,
            Some(Level::Medium) => self.medium += 1,
            Some(Level::Low) => self.low += 1,
            None => self.unscored += 1,
        }
    }
}

/// Mitigation counts by resolution status (spec §6.3), across every
/// failure mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionCounts {
    pub planned: usize,
    pub implemented: usize,
    pub verified: usize,
    pub abandoned: usize,
}

/// The verified gate: the same readiness computation run on the views with
/// every non-verified mitigation removed. Sign-off at the `verification`
/// level uses `verified.ready`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedGate {
    pub ready: bool,
    pub blockers: Vec<String>,
}

/// The computed readiness report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessReport {
    pub ready: bool,
    /// Residual-criticality buckets across all non-excluded failure modes.
    pub by_criticality: CriticalityBuckets,
    /// Human-readable blockers; empty iff `ready`.
    pub blockers: Vec<String>,
    /// Mitigations by resolution status.
    #[serde(default)]
    pub resolution: ResolutionCounts,
    /// The gate computed from verified mitigations only.
    #[serde(default)]
    pub verified: VerifiedGate,
    /// Failure-mode ids with disposition `Accepted`.
    #[serde(default)]
    pub accepted_risks: Vec<String>,
    /// Failure-mode ids with disposition `Excluded`.
    #[serde(default)]
    pub excluded: Vec<String>,
    /// Failure-mode ids that are `single_point`, raw criticality High, and not
    /// excluded.
    #[serde(default)]
    pub critical_items: Vec<String>,
}

/// One run of the gate over a set of views.
struct Gate {
    by_criticality: CriticalityBuckets,
    blockers: Vec<String>,
}

/// The readiness gate over `failure_modes`. `Excluded` failure modes are left
/// out entirely (buckets and blockers); `Accepted` ones are counted in the
/// buckets but raise no blocker.
fn gate(failure_modes: &[FailureModeView], issues: &[Issue]) -> Gate {
    let mut by_criticality = CriticalityBuckets::default();
    let mut blockers = Vec::new();
    let considered: Vec<&FailureModeView> = failure_modes
        .iter()
        .filter(|fmv| fmv.disposition != Disposition::Excluded)
        .collect();
    let blocking = |fmv: &&&FailureModeView| fmv.disposition == Disposition::Open;

    // (1) Every failure mode must be scored (cause+effect+score).
    for fmv in &considered {
        by_criticality.record(fmv.residual_criticality);
    }
    for fmv in considered.iter().filter(blocking) {
        if !fmv.is_scored() {
            blockers.push(format!(
                "CLARIFY: failure mode '{}' is missing cause/effect/score",
                fmv.failure_mode.id
            ));
        }
    }

    // (2) No residual High/Medium remaining.
    for fmv in considered.iter().filter(blocking) {
        match fmv.residual_criticality {
            Some(Level::High) => blockers.push(format!(
                "RESIDUAL: failure mode '{}' still has High residual criticality",
                fmv.failure_mode.id
            )),
            Some(Level::Medium) => blockers.push(format!(
                "RESIDUAL: failure mode '{}' still has Medium residual criticality",
                fmv.failure_mode.id
            )),
            _ => {}
        }
    }

    // (3) No standing weak_mitigation_order issue on an open failure mode.
    for issue in issues {
        let open = considered
            .iter()
            .filter(blocking)
            .any(|fmv| fmv.failure_mode.id == issue.failure_mode_id);
        if open && issue.r#type == IssueType::WeakMitigationOrder {
            blockers.push(format!(
                "DISCIPLINE: failure mode '{}' relies only on fail_fast (weak mitigation order)",
                issue.failure_mode_id
            ));
        }
    }

    // An FMECA with no failure modes is not "ready" — there is nothing analyzed.
    if considered.is_empty() {
        blockers.push("EMPTY: no failure modes have been analyzed".to_string());
    }

    Gate {
        by_criticality,
        blockers,
    }
}

/// Ids of the failure modes with the given disposition, in insertion order.
fn ids_with(failure_modes: &[FailureModeView], disposition: Disposition) -> Vec<String> {
    failure_modes
        .iter()
        .filter(|fmv| fmv.disposition == disposition)
        .map(|fmv| fmv.failure_mode.id.clone())
        .collect()
}

fn resolution_counts(failure_modes: &[FailureModeView]) -> ResolutionCounts {
    let mut counts = ResolutionCounts::default();
    for m in failure_modes.iter().flat_map(|fmv| &fmv.mitigations) {
        match m.status {
            MitigationStatus::Planned => counts.planned += 1,
            MitigationStatus::Implemented => counts.implemented += 1,
            MitigationStatus::Verified => counts.verified += 1,
            MitigationStatus::Abandoned => counts.abandoned += 1,
        }
    }
    counts
}

/// Compute the readiness report deterministically. Pure over its
/// inputs — the same projection always yields the same report.
/// `verified_views` / `verified_issues` are the views with every
/// non-verified mitigation removed, and the issues detected on them.
pub(crate) fn assess(
    failure_modes: &[FailureModeView],
    issues: &[Issue],
    verified_views: &[FailureModeView],
    verified_issues: &[Issue],
) -> ReadinessReport {
    let declared = gate(failure_modes, issues);
    let verified = gate(verified_views, verified_issues);
    let critical_items = failure_modes
        .iter()
        .filter(|fmv| {
            fmv.failure_mode.single_point
                && fmv.criticality == Some(Level::High)
                && fmv.disposition != Disposition::Excluded
        })
        .map(|fmv| fmv.failure_mode.id.clone())
        .collect();

    ReadinessReport {
        ready: declared.blockers.is_empty(),
        by_criticality: declared.by_criticality,
        blockers: declared.blockers,
        resolution: resolution_counts(failure_modes),
        verified: VerifiedGate {
            ready: verified.blockers.is_empty(),
            blockers: verified.blockers,
        },
        accepted_risks: ids_with(failure_modes, Disposition::Accepted),
        excluded: ids_with(failure_modes, Disposition::Excluded),
        critical_items,
    }
}

impl Default for ReadinessReport {
    fn default() -> Self {
        Self {
            ready: false,
            by_criticality: CriticalityBuckets::default(),
            blockers: vec!["EMPTY: no events".to_string()],
            resolution: ResolutionCounts::default(),
            verified: VerifiedGate {
                ready: false,
                blockers: vec!["EMPTY: no events".to_string()],
            },
            accepted_risks: Vec::new(),
            excluded: Vec::new(),
            critical_items: Vec::new(),
        }
    }
}
