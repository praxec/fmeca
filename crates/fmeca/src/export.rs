//! `report.export`: emit the FMECA report.
//!
//! A row per failure mode (the FMECA table per `P_RUN_FMECA`'s output schema),
//! the residual-risk buckets, and an explicit **accepted-risks** section. v1 has
//! no "accept-risk" move, so `accepted_risks` is always empty — it exists so the
//! report schema is stable and nothing High/Medium is ever waved through
//! silently.

use serde::{Deserialize, Serialize};

use crate::model::{Criticality, Domain, FailureModeStanding, MitigationKind};
use crate::projection::FmecaState;
use crate::readiness::CriticalityBuckets;
use crate::response::ResponseClass;

/// One mitigation as it appears in an exported FMECA row. The residual
/// criticality is DERIVED from the mitigation's residual observations via the
/// SAME scoring + matrix the projection uses — never from a
/// caller-supplied level. `None` when its residual axes are unobserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedMitigation {
    pub id: String,
    pub kind: MitigationKind,
    pub description: String,
    pub residual_criticality: Option<Criticality>,
}

/// A single FMECA table row (`P_RUN_FMECA` schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FmecaRow {
    pub failure_mode_id: String,
    pub component: String,
    pub domain: Domain,
    pub description: String,
    pub cause: Option<String>,
    pub effect: Option<String>,
    pub criticality: Option<Criticality>,
    pub residual_criticality: Option<Criticality>,
    pub standing: Option<FailureModeStanding>,
    /// Deterministically-derived remediation magnitude.
    pub response_class: Option<ResponseClass>,
    pub mitigations: Vec<ExportedMitigation>,
    pub source_turn: String,
}

/// The exported FMECA report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FmecaReport {
    pub session_id: String,
    pub ready: bool,
    pub rows: Vec<FmecaRow>,
    pub residual_buckets: CriticalityBuckets,
    /// Explicitly accepted residual risks. Empty in v1 (no accept-risk move).
    pub accepted_risks: Vec<String>,
    /// Carried verbatim so the caller sees exactly why it is/isn't ready.
    pub blockers: Vec<String>,
}

/// Build the exported report from a projection (deterministic).
pub(crate) fn build(state: &FmecaState) -> FmecaReport {
    let rows = state
        .failure_modes
        .iter()
        .map(|fmv| {
            let fm = &fmv.failure_mode;
            FmecaRow {
                failure_mode_id: fm.id.clone(),
                component: fm.component.id.clone(),
                domain: fm.domain,
                description: fm.description.clone(),
                cause: fm.cause.clone(),
                effect: fm.effect.clone(),
                criticality: fmv.criticality,
                residual_criticality: fmv.residual_criticality,
                standing: fmv.standing,
                response_class: fmv.response_class,
                mitigations: fmv
                    .mitigations
                    .iter()
                    .map(|m| ExportedMitigation {
                        id: m.id.clone(),
                        kind: m.kind,
                        description: m.description.clone(),
                        // SAME residual derivation as the projection:
                        // observations → ordinal → strategy matrix. No
                        // second scoring path.
                        residual_criticality: crate::projection::mitigation_residual(
                            state.matrix_strategy,
                            m,
                        ),
                    })
                    .collect(),
                source_turn: fm.source.turn_id.clone(),
            }
        })
        .collect();

    FmecaReport {
        session_id: state.session_id.clone(),
        ready: state.readiness.ready,
        rows,
        residual_buckets: state.readiness.by_criticality.clone(),
        accepted_risks: Vec::new(),
        blockers: state.readiness.blockers.clone(),
    }
}
