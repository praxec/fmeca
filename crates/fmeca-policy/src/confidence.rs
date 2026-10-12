//! The evidence confidence ladder (spec §4.3).
//!
//! | Level | Evidence that reaches it |
//! |---|---|
//! | C0 | no evidence, or only `note` items |
//! | C1 | basis `design`; or kind `file` / `url` without a basis |
//! | C2 | basis `inspection`; or kind `pull_request` / `commit` |
//! | C3 | basis `test`, or kind `test` |
//! | C4 | basis `operational_use` or `failure_history` |

use fmeca::{EvidenceBasis, EvidenceKind, ResolutionEvidence};

use crate::model::Confidence;
use crate::subject::SubjectFm;

/// The confidence one evidence item reaches. A `note` is C0 whatever its
/// basis; otherwise the higher of what the kind and the basis reach.
pub fn evidence_confidence(e: &ResolutionEvidence) -> Confidence {
    let by_kind = match e.kind {
        EvidenceKind::Note => return Confidence::C0,
        EvidenceKind::File | EvidenceKind::Url => Confidence::C1,
        EvidenceKind::PullRequest | EvidenceKind::Commit => Confidence::C2,
        EvidenceKind::Test => Confidence::C3,
    };
    let by_basis = match e.basis {
        None => Confidence::C0,
        Some(EvidenceBasis::Design) => Confidence::C1,
        Some(EvidenceBasis::Inspection) => Confidence::C2,
        Some(EvidenceBasis::Test) => Confidence::C3,
        Some(EvidenceBasis::OperationalUse | EvidenceBasis::FailureHistory) => Confidence::C4,
    };
    by_kind.max(by_basis)
}

/// The confidence in a failure mode's residual: the best evidence item over
/// the active mitigations that achieve [`SubjectFm::residual`]. C0 when no
/// mitigation achieves it (including a failure mode with no mitigations).
pub fn residual_confidence(fm: &SubjectFm) -> Confidence {
    let Some(residual) = fm.residual else {
        return Confidence::C0;
    };
    fm.mitigations
        .iter()
        .filter(|m| m.is_active() && m.residual == Some(residual))
        .flat_map(|m| m.evidence.iter())
        .map(evidence_confidence)
        .max()
        .unwrap_or(Confidence::C0)
}
