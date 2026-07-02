//! The readiness gate: deterministic, computed, honest.
//!
//! `ready == true` **iff** all three conditions hold:
//!  1. Every failure mode has cause + effect + score (no `clarify` / no
//!     `missing_*` issue).
//!  2. No failure mode has a **residual** criticality of High or Medium (all
//!     reduced to Low).
//!  3. No `weak_mitigation_order` issue stands. v1 has no
//!     "accept-risk" move, so any such issue blocks; the report surfaces it so
//!     nothing is waved through silently.
//!
//! The report **shows its work**: residual buckets + every blocker.

use serde::{Deserialize, Serialize};

use crate::model::{Criticality, Issue, IssueType, Level};
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

/// The computed readiness report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessReport {
    pub ready: bool,
    /// Residual-criticality buckets across all failure modes.
    pub by_criticality: CriticalityBuckets,
    /// Human-readable blockers; empty iff `ready`.
    pub blockers: Vec<String>,
}

/// Compute the readiness report deterministically. Pure over its
/// inputs — the same projection always yields the same report.
pub(crate) fn assess(failure_modes: &[FailureModeView], issues: &[Issue]) -> ReadinessReport {
    let mut by_criticality = CriticalityBuckets::default();
    let mut blockers = Vec::new();

    // (1) Every failure mode must be scored (cause+effect+score).
    for fmv in failure_modes {
        by_criticality.record(fmv.residual_criticality);
        if !fmv.is_scored() {
            blockers.push(format!(
                "CLARIFY: failure mode '{}' is missing cause/effect/score",
                fmv.failure_mode.id
            ));
        }
    }

    // (2) No residual High/Medium remaining.
    for fmv in failure_modes {
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

    // (3) No standing weak_mitigation_order issue.
    for issue in issues {
        if issue.r#type == IssueType::WeakMitigationOrder {
            blockers.push(format!(
                "DISCIPLINE: failure mode '{}' relies only on fail_fast (weak mitigation order)",
                issue.failure_mode_id
            ));
        }
    }

    // An FMECA with no failure modes is not "ready" — there is nothing analyzed.
    if failure_modes.is_empty() {
        blockers.push("EMPTY: no failure modes have been analyzed".to_string());
    }

    ReadinessReport {
        ready: blockers.is_empty(),
        by_criticality,
        blockers,
    }
}

impl Default for ReadinessReport {
    fn default() -> Self {
        Self {
            ready: false,
            by_criticality: CriticalityBuckets::default(),
            blockers: vec!["EMPTY: no events".to_string()],
        }
    }
}
