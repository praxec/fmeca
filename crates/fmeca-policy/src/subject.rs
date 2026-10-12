//! [`Subject`]: one shape for what the evaluator reads, built either from a
//! projected session ([`subject_from_state`]) or from a one-shot analysis
//! ([`subject_from_analysis`]).
//!
//! Severity and probability are carried as the strategy's 1-based *ordinals*
//! (`(severity, probability)`), derived from observations exactly as the
//! kernel derives them ([`fmeca::derive_ordinal`]), so a policy matrix cell is
//! `cells[severity - 1][probability - 1]`.

use fmeca::{
    AnalysisLevel, AnalyzeInput, Axis, Disposition, Domain, FailureModeView, FmecaError,
    FmecaState, MatrixStrategy, Mitigation, MitigationKind, MitigationStatus, ResolutionEvidence,
    ResponseClass, derive_ordinal,
};

/// What the evaluator reads: the analysis's strategy and level, its failure
/// modes, and the kernel's readiness gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    /// The analysis's matrix strategy; selects the policy matrix.
    pub strategy: MatrixStrategy,
    /// The analysis level, when recorded.
    pub level: Option<AnalysisLevel>,
    /// Every failure mode, excluded ones included.
    pub failure_modes: Vec<SubjectFm>,
    /// The kernel's readiness gate (`readiness.assess`).
    pub kernel_gate: KernelGate,
}

/// The kernel's readiness verdict, carried so a policy's sign-off can include
/// it (`signoff.include_kernel_gate`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KernelGate {
    /// The kernel's `ready`.
    pub ready: bool,
    /// The kernel's human-readable blockers; empty iff `ready`.
    pub blockers: Vec<String>,
}

/// One failure mode as the evaluator sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectFm {
    /// Failure-mode id.
    pub id: String,
    /// The failure mode's domain.
    pub domain: Domain,
    /// Whether it is a single point of failure.
    pub single_point: bool,
    /// Unmitigated `(severity, probability)` ordinals; `None` when unscored.
    pub inherent: Option<(u8, u8)>,
    /// The best residual `(severity, probability)` ordinals over the active
    /// mitigations, chosen as the kernel chooses (lowest criticality, first in
    /// append order); `None` when no active mitigation yields a residual.
    pub residual: Option<(u8, u8)>,
    /// The kernel's derived response class; `None` when unscored.
    pub response_class: Option<ResponseClass>,
    /// The recorded disposition.
    pub disposition: Disposition,
    /// The rationale recorded with the current disposition, if any (the
    /// exclusion rationale for an excluded failure mode).
    pub disposition_rationale: Option<String>,
    /// The acceptance, when the disposition is `Accepted`.
    pub accepted: Option<Accepted>,
    /// Every mitigation, abandoned ones included, in append order.
    pub mitigations: Vec<SubjectMitigation>,
}

/// A recorded acceptance: the last disposition change to `Accepted`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// Who accepted the risk (empty when not recorded).
    pub approver: String,
    /// When, as RFC 3339 (stamped by the server).
    pub at: Option<String>,
    /// Why.
    pub rationale: String,
}

/// One mitigation as the evaluator sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectMitigation {
    /// Mitigation id.
    pub id: String,
    /// Prevention, detection or fail-fast.
    pub kind: MitigationKind,
    /// Resolution status; `Abandoned` mitigations are ignored.
    pub status: MitigationStatus,
    /// Residual `(severity, probability)` ordinals; `None` when either
    /// residual axis is unobserved.
    pub residual: Option<(u8, u8)>,
    /// Evidence accumulated across the mitigation's status changes.
    pub evidence: Vec<ResolutionEvidence>,
}

impl SubjectMitigation {
    /// True unless the mitigation was abandoned.
    pub fn is_active(&self) -> bool {
        self.status != MitigationStatus::Abandoned
    }
}

/// Build a [`Subject`] from a projected session.
pub fn subject_from_state(state: &FmecaState) -> Subject {
    let strategy = state.matrix_strategy;
    Subject {
        strategy,
        level: state.meta.level,
        failure_modes: state
            .failure_modes
            .iter()
            .map(|fm| subject_fm_from_view(strategy, fm))
            .collect(),
        kernel_gate: KernelGate {
            ready: state.readiness.ready,
            blockers: state.readiness.blockers.clone(),
        },
    }
}

/// Build a [`Subject`] from a one-shot analysis. Runs [`fmeca::analyze()`] for
/// the response classes and the readiness gate, so an unknown observation id
/// is the kernel's [`FmecaError::InvalidObservation`].
///
/// A one-shot analysis has no level, no single-point flags, no dispositions
/// (every failure mode is open) and no resolution evidence (every mitigation
/// is planned).
pub fn subject_from_analysis(
    strategy: MatrixStrategy,
    input: &[AnalyzeInput],
) -> Result<Subject, FmecaError> {
    let report = fmeca::analyze(strategy, input)?;
    let mut failure_modes = Vec::with_capacity(input.len());
    for fm in input {
        let mitigations = fm
            .mitigations
            .iter()
            .map(|m| {
                Ok(SubjectMitigation {
                    id: m.id.clone(),
                    kind: m.kind,
                    status: MitigationStatus::Planned,
                    residual: ordinals(
                        derive_ordinal(
                            strategy,
                            Axis::Severity,
                            &m.residual_severity_observations,
                        )?,
                        derive_ordinal(
                            strategy,
                            Axis::Probability,
                            &m.residual_probability_observations,
                        )?,
                    ),
                    evidence: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, FmecaError>>()?;
        let response_class = report
            .failure_modes
            .iter()
            .find(|r| r.id == fm.id)
            .and_then(|r| r.response_class);
        failure_modes.push(SubjectFm {
            id: fm.id.clone(),
            domain: fm.domain,
            single_point: false,
            inherent: ordinals(
                derive_ordinal(strategy, Axis::Severity, &fm.severity_observations)?,
                derive_ordinal(strategy, Axis::Probability, &fm.probability_observations)?,
            ),
            residual: best_residual(strategy, &mitigations),
            response_class,
            disposition: Disposition::Open,
            disposition_rationale: None,
            accepted: None,
            mitigations,
        });
    }
    Ok(Subject {
        strategy,
        level: None,
        failure_modes,
        kernel_gate: KernelGate {
            ready: report.ready,
            blockers: report.blockers,
        },
    })
}

fn subject_fm_from_view(strategy: MatrixStrategy, fm: &FailureModeView) -> SubjectFm {
    let mitigations: Vec<SubjectMitigation> = fm
        .mitigations
        .iter()
        .map(|m| SubjectMitigation {
            id: m.id.clone(),
            kind: m.kind,
            status: m.status,
            residual: mitigation_residual(strategy, m),
            evidence: m.evidence.clone(),
        })
        .collect();
    let last_change = fm.disposition_history.last();
    let accepted = (fm.disposition == Disposition::Accepted)
        .then(|| {
            last_change.map(|c| Accepted {
                approver: c.approver.clone().unwrap_or_default(),
                at: c.at.clone(),
                rationale: c.rationale.clone(),
            })
        })
        .flatten();
    SubjectFm {
        id: fm.failure_mode.id.clone(),
        domain: fm.failure_mode.domain,
        single_point: fm.failure_mode.single_point,
        inherent: ordinals(
            fm.derived_severity.as_ref().map(|l| l.ordinal),
            fm.derived_probability.as_ref().map(|l| l.ordinal),
        ),
        residual: best_residual(strategy, &mitigations),
        response_class: fm.response_class,
        disposition: fm.disposition,
        disposition_rationale: last_change.map(|c| c.rationale.clone()),
        accepted,
        mitigations,
    }
}

/// A mitigation's residual ordinals, derived from its observations. Persisted
/// observations were validated at write time; an unknown id yields `None`, as
/// in the kernel's replay.
fn mitigation_residual(strategy: MatrixStrategy, m: &Mitigation) -> Option<(u8, u8)> {
    ordinals(
        derive_ordinal(strategy, Axis::Severity, &m.residual_severity_observations)
            .ok()
            .flatten(),
        derive_ordinal(
            strategy,
            Axis::Probability,
            &m.residual_probability_observations,
        )
        .ok()
        .flatten(),
    )
}

fn ordinals(severity: Option<u8>, probability: Option<u8>) -> Option<(u8, u8)> {
    Some((severity?, probability?))
}

/// The kernel's residual choice: over the active mitigations with a residual,
/// the lowest criticality, first in append order on ties.
fn best_residual(strategy: MatrixStrategy, mitigations: &[SubjectMitigation]) -> Option<(u8, u8)> {
    mitigations
        .iter()
        .filter(|m| m.is_active())
        .filter_map(|m| m.residual)
        .min_by_key(|&(s, p)| strategy.criticality(s, p).rank())
}
