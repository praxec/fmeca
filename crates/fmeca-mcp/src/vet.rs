//! The advisory `vet` tool: a calibrated second opinion from Jev beside the
//! deterministic results, never instead of them.
//!
//! `vet` reads a session (or a one-shot `analyze`-shaped analysis), plans a
//! deterministic list of questions in four checks, sends the first
//! `max_questions` of them to a [`JudgmentModel`] in one batched call, and
//! reports one finding per answered question with its probability, the
//! threshold it was judged against and a verdict. It never appends to a log
//! and never changes a score: the `readiness_gate` it echoes is the kernel's.
//!
//! # Checks
//!
//! | Check          | Question | Target | Flag when |
//! |----------------|----------|--------|-----------|
//! | `observations` | Noul per claimed observation | `<fm>/<obs>` | `p <` [`CLAIMED_OBS_MIN`] |
//! | `observations` | Noul per axis for the next level up (unclaimed) | `<fm>/<obs>` | `p >` [`UNCLAIMED_OBS_MAX`] |
//! | `mitigations`  | Score over [`SCORE_LEVELS`]: how fully it addresses the cause | `<fm>/<m>` | `p <` [`MITIGATION_ADDRESSES_MIN`] |
//! | `mitigations`  | Choice over the mitigation kinds | `<fm>/<m>#class` | argmax ≠ declared and argmax `p ≥` [`MITIGATION_CLASS_CONFIDENCE`] |
//! | `coverage`     | Score per [`FAILURE_CLASSES`] entry, once per analysis | `coverage/<class>` | `p ≥` [`COVERAGE_GAP_MIN`] |
//! | `readiness`    | Score: sound enough to sign off? | `readiness` | `p <` [`READINESS_MIN`] (verdict `concern`) |
//!
//! A Noul's `p` is its `noul`; a Score's is `score / 4`; a Choice's is the
//! probability of the declared kind. Failure modes dispositioned `excluded`
//! (not credible) are left out of the subject entirely.
//!
//! # Order and cap
//!
//! The analysis-wide questions are planned first, so the cap never starves
//! them: readiness (one question), then coverage (one per
//! [`FAILURE_CLASSES`] entry, in that order). Then come the per-failure-mode
//! questions, failure mode by failure mode (sorted by id): observations,
//! then mitigations, each in id order. `checks` filters which are planned.
//! The first `max_questions` are sent; the rest are listed in `skipped` by
//! hash. Hashes do not depend on position.
//!
//! # Determinism
//!
//! A question's id is [`question_hash`]: the sha256 of the canonical JSON of
//! its check, target, question and the state sent to Jev, so the same
//! subject always yields the same ids.

use std::collections::{BTreeMap, BTreeSet};

use fmeca::{
    AnalyzeInput, Axis, Disposition, Domain, FmecaError, FmecaState, MatrixStrategy,
    MitigationKind, ScoreCriterion, catalog_for,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::jev::{Answer, JudgmentModel, Question, Unavailable, UnavailableReason};

/// Questions sent when the caller does not pass `max_questions`.
pub const DEFAULT_MAX_QUESTIONS: usize = 40;
/// The largest accepted `max_questions`; above it the call is rejected.
pub const HARD_MAX_QUESTIONS: usize = 200;
/// The checks, in planning order.
pub const CHECKS: [&str; 4] = ["observations", "mitigations", "coverage", "readiness"];
/// The failure classes the `coverage` check asks about.
pub const FAILURE_CLASSES: [&str; 7] = [
    "data_integrity",
    "concurrency",
    "external_dependency",
    "security",
    "resource_exhaustion",
    "configuration",
    "operator_error",
];
/// The five levels of every Score question; `score / 4` is its probability.
pub const SCORE_LEVELS: [&str; 5] = ["not at all", "slightly", "partly", "mostly", "fully"];

/// A claimed observation is flagged when Jev's probability is below this.
pub const CLAIMED_OBS_MIN: f64 = 0.3;
/// An unclaimed observation is flagged when Jev's probability is above this.
pub const UNCLAIMED_OBS_MAX: f64 = 0.7;
/// A mitigation is flagged when it addresses its cause less than this.
pub const MITIGATION_ADDRESSES_MIN: f64 = 0.5;
/// A mitigation's class is flagged when another kind is this likely or more.
pub const MITIGATION_CLASS_CONFIDENCE: f64 = 0.6;
/// A failure class is a coverage gap when Jev's probability is this or more.
pub const COVERAGE_GAP_MIN: f64 = 0.6;
/// Readiness is a concern when Jev's probability is below this.
pub const READINESS_MIN: f64 = 0.5;

const CHECK_OBSERVATIONS: &str = CHECKS[0];
const CHECK_MITIGATIONS: &str = CHECKS[1];
const CHECK_COVERAGE: &str = CHECKS[2];
const CHECK_READINESS: &str = CHECKS[3];

/// Shown where a failure mode has no cause or effect yet.
const NOT_STATED: &str = "(not stated)";

/// `vet` arguments: exactly one of `session_id` and `analysis`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VetArgs {
    /// The session to vet.
    pub session_id: Option<String>,
    /// A one-shot analysis to vet, in the `analyze` tool's argument shape.
    pub analysis: Option<VetAnalysis>,
    /// The checks to run (default: all of [`CHECKS`]).
    pub checks: Option<Vec<String>>,
    /// How many questions to send (default [`DEFAULT_MAX_QUESTIONS`], at
    /// most [`HARD_MAX_QUESTIONS`]).
    pub max_questions: Option<usize>,
}

/// The `analyze` tool's argument shape, vetted without a session.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VetAnalysis {
    /// The criticality matrix (default 3×3).
    #[serde(default)]
    pub matrix_strategy: MatrixStrategy,
    /// The whole batch of failure modes.
    pub failure_modes: Vec<AnalyzeInput>,
}

/// Normalised view of what is being vetted (session or one-shot analysis).
#[derive(Debug, Clone)]
pub struct VetSubject {
    /// The strategy whose catalog the observations come from.
    pub strategy: MatrixStrategy,
    /// The failure modes, sorted by id, without excluded ones.
    pub failure_modes: Vec<SubjectFailureMode>,
    /// The kernel's readiness gate, echoed verbatim.
    pub readiness_gate: Value,
}

/// One failure mode being vetted.
#[derive(Debug, Clone)]
pub struct SubjectFailureMode {
    pub id: String,
    pub component: String,
    pub description: String,
    pub cause: Option<String>,
    pub effect: Option<String>,
    pub domain: Domain,
    pub severity_observations: Vec<String>,
    pub probability_observations: Vec<String>,
    /// Sorted by id.
    pub mitigations: Vec<SubjectMitigation>,
}

/// One mitigation being vetted.
#[derive(Debug, Clone)]
pub struct SubjectMitigation {
    pub id: String,
    pub kind: MitigationKind,
    pub description: String,
}

/// What a planned question asks, which decides how its answer is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedKind {
    /// Does a claimed observation apply? (Noul)
    ClaimedObservation,
    /// Does the next observation up apply? (Noul)
    UnclaimedObservation,
    /// How fully does a mitigation address the cause? (Score)
    MitigationAddresses,
    /// Which kind is a mitigation, against its declared kind? (Choice)
    MitigationClass { declared: MitigationKind },
    /// Is a failure class uncovered? (Score)
    Coverage,
    /// Is the analysis sound enough to sign off? (Score)
    Readiness,
}

/// One planned question.
#[derive(Debug, Clone)]
pub struct Planned {
    /// The [`question_hash`]; also the question id sent to Jev.
    pub id: String,
    /// One of [`CHECKS`].
    pub check: &'static str,
    /// What the question is about (`<fm>/<obs>`, `<fm>/<m>`, …).
    pub target: String,
    /// How to read the answer.
    pub kind: PlannedKind,
    /// The question sent to Jev.
    pub question: Question,
}

/// The subject of a session: its failure modes except excluded ones, their
/// non-abandoned mitigations, and its readiness report as the gate.
pub fn subject_from_state(state: &FmecaState) -> VetSubject {
    let mut failure_modes: Vec<SubjectFailureMode> = state
        .failure_modes
        .iter()
        .filter(|fmv| fmv.disposition != Disposition::Excluded)
        .map(|fmv| {
            let fm = &fmv.failure_mode;
            SubjectFailureMode {
                id: fm.id.clone(),
                component: fm.component.id.clone(),
                description: fm.description.clone(),
                cause: fm.cause.clone(),
                effect: fm.effect.clone(),
                domain: fm.domain,
                severity_observations: fm.severity_observations.clone(),
                probability_observations: fm.probability_observations.clone(),
                mitigations: sorted_mitigations(fmv.active_mitigations().map(|m| {
                    SubjectMitigation {
                        id: m.id.clone(),
                        kind: m.kind,
                        description: m.description.clone(),
                    }
                })),
            }
        })
        .collect();
    failure_modes.sort_by(|a, b| a.id.cmp(&b.id));
    VetSubject {
        strategy: state.matrix_strategy,
        failure_modes,
        readiness_gate: serde_json::to_value(&state.readiness).unwrap_or(Value::Null),
    }
}

/// The subject of a one-shot analysis. Runs [`fmeca::analyze()`] for the gate,
/// so an unknown observation id is the same `INVALID_OBSERVATION` error.
pub fn subject_from_analysis(
    strategy: MatrixStrategy,
    fms: &[AnalyzeInput],
) -> Result<VetSubject, FmecaError> {
    let report = fmeca::analyze(strategy, fms)?;
    let mut failure_modes: Vec<SubjectFailureMode> = fms
        .iter()
        .map(|fm| SubjectFailureMode {
            id: fm.id.clone(),
            component: fm.component.id.clone(),
            description: fm.description.clone(),
            cause: fm.cause.clone(),
            effect: fm.effect.clone(),
            domain: fm.domain,
            severity_observations: fm.severity_observations.clone(),
            probability_observations: fm.probability_observations.clone(),
            mitigations: sorted_mitigations(fm.mitigations.iter().map(|m| SubjectMitigation {
                id: m.id.clone(),
                kind: m.kind,
                description: m.description.clone(),
            })),
        })
        .collect();
    failure_modes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(VetSubject {
        strategy,
        failure_modes,
        readiness_gate: json!({ "ready": report.ready, "blockers": report.blockers }),
    })
}

fn sorted_mitigations(ms: impl Iterator<Item = SubjectMitigation>) -> Vec<SubjectMitigation> {
    let mut ms: Vec<SubjectMitigation> = ms.collect();
    ms.sort_by(|a, b| a.id.cmp(&b.id));
    ms
}

/// The state sent to Jev: the failure modes with their claimed observations
/// (id, axis and catalog description) and their mitigations.
pub fn subject_state(subject: &VetSubject) -> Value {
    let catalog = catalog_for(subject.strategy);
    let observations = |ids: &[String]| -> Vec<Value> {
        ids.iter()
            .map(|id| {
                json!({
                    "id": id,
                    "description": criterion(&catalog, id).map(|c| c.description.clone()),
                })
            })
            .collect()
    };
    let failure_modes: Vec<Value> = subject
        .failure_modes
        .iter()
        .map(|fm| {
            json!({
                "id": fm.id,
                "component": fm.component,
                "description": fm.description,
                "cause": fm.cause,
                "effect": fm.effect,
                "domain": fm.domain,
                "severity_observations": observations(&fm.severity_observations),
                "probability_observations": observations(&fm.probability_observations),
                "mitigations": fm.mitigations.iter().map(|m| json!({
                    "id": m.id,
                    "kind": m.kind,
                    "description": m.description,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "matrix_strategy": subject.strategy, "failure_modes": failure_modes })
}

fn criterion<'a>(catalog: &'a [ScoreCriterion], id: &str) -> Option<&'a ScoreCriterion> {
    catalog.iter().find(|c| c.id == id)
}

/// The sha256 hex of the canonical JSON (object keys sorted at every depth)
/// of `{check, target, question, state}`.
pub fn question_hash(check: &str, target: &str, question: &Question, state: &Value) -> String {
    hash_with_state_bytes(check, target, question, &canonical_bytes(state))
}

/// [`question_hash`] over the already-canonical bytes of the state, so a
/// plan serialises its (possibly large) state once, not once per question.
/// The document is written by hand in the canonical key order
/// (`check`, `question`, `state`, `target`), byte-identical to
/// `serde_json::to_vec` of the canonical object.
fn hash_with_state_bytes(
    check: &str,
    target: &str,
    question: &Question,
    state_bytes: &[u8],
) -> String {
    let question = serde_json::to_value(question).unwrap_or(Value::Null);
    let mut hasher = Sha256::new();
    hasher.update(b"{\"check\":");
    hasher.update(canonical_bytes(&json!(check)));
    hasher.update(b",\"question\":");
    hasher.update(canonical_bytes(&question));
    hasher.update(b",\"state\":");
    hasher.update(state_bytes);
    hasher.update(b",\"target\":");
    hasher.update(canonical_bytes(&json!(target)));
    hasher.update(b"}");
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn canonical_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&canonical(value)).unwrap_or_default()
}

/// `value` with every object's keys in sorted order, whatever the
/// `serde_json` map implementation (`preserve_order` is on in this tree).
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> =
                map.iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Every question for `checks`, in planning order (see the module docs).
/// Deterministic and uncapped; a repeated hash is planned once.
pub fn plan_questions(subject: &VetSubject, checks: &[&str]) -> Vec<Planned> {
    let state_bytes = canonical_bytes(&subject_state(subject));
    let catalog = catalog_for(subject.strategy);
    let wants = |check: &str| checks.contains(&check);
    let mut drafts: Vec<(&'static str, String, PlannedKind, Question)> = Vec::new();

    // Analysis-wide questions first, so the cap never starves them.
    if wants(CHECK_READINESS) {
        drafts.push((
            CHECK_READINESS,
            "readiness".to_string(),
            PlannedKind::Readiness,
            score("Is this analysis sound enough to sign off?".to_string()),
        ));
    }
    if wants(CHECK_COVERAGE) {
        for class in FAILURE_CLASSES {
            drafts.push((
                CHECK_COVERAGE,
                format!("coverage/{class}"),
                PlannedKind::Coverage,
                score(format!(
                    "Does the class '{class}' plausibly apply to this analysis but is not \
                     covered by any failure mode?"
                )),
            ));
        }
    }

    let mut fms: Vec<&SubjectFailureMode> = subject.failure_modes.iter().collect();
    fms.sort_by(|a, b| a.id.cmp(&b.id));
    for fm in fms {
        if wants(CHECK_OBSERVATIONS) {
            observation_questions(fm, &catalog, &mut drafts);
        }
        if wants(CHECK_MITIGATIONS) {
            mitigation_questions(fm, &mut drafts);
        }
    }

    let mut seen = BTreeSet::new();
    drafts
        .into_iter()
        .filter_map(|(check, target, kind, question)| {
            let id = hash_with_state_bytes(check, &target, &question, &state_bytes);
            seen.insert(id.clone()).then_some(Planned {
                id,
                check,
                target,
                kind,
                question,
            })
        })
        .collect()
}

type Draft = (&'static str, String, PlannedKind, Question);

fn observation_questions(
    fm: &SubjectFailureMode,
    catalog: &[ScoreCriterion],
    drafts: &mut Vec<Draft>,
) {
    let applies = |obs: &str| {
        let description = criterion(catalog, obs).map_or(obs, |c| c.description.as_str());
        noul(format!(
            "Given cause '{}' and effect '{}' of '{}', does the observation '{description}' apply?",
            fm.cause.as_deref().unwrap_or(NOT_STATED),
            fm.effect.as_deref().unwrap_or(NOT_STATED),
            fm.description,
        ))
    };
    let claimed: BTreeSet<&String> = fm
        .severity_observations
        .iter()
        .chain(&fm.probability_observations)
        .collect();
    for obs in &claimed {
        drafts.push((
            CHECK_OBSERVATIONS,
            format!("{}/{obs}", fm.id),
            PlannedKind::ClaimedObservation,
            applies(obs),
        ));
    }
    for axis in [Axis::Severity, Axis::Probability] {
        let on_axis: Vec<&ScoreCriterion> = catalog.iter().filter(|c| c.axis == axis).collect();
        let max = on_axis
            .iter()
            .filter(|c| claimed.contains(&c.id))
            .map(|c| c.level_ordinal)
            .max()
            .unwrap_or(0);
        let next = on_axis
            .iter()
            .filter(|c| u16::from(c.level_ordinal) == u16::from(max) + 1)
            .min_by(|a, b| a.id.cmp(&b.id));
        if let Some(next) = next {
            drafts.push((
                CHECK_OBSERVATIONS,
                format!("{}/{}", fm.id, next.id),
                PlannedKind::UnclaimedObservation,
                applies(&next.id),
            ));
        }
    }
}

fn mitigation_questions(fm: &SubjectFailureMode, drafts: &mut Vec<Draft>) {
    let cause = fm.cause.as_deref().unwrap_or(NOT_STATED);
    for m in &fm.mitigations {
        drafts.push((
            CHECK_MITIGATIONS,
            format!("{}/{}", fm.id, m.id),
            PlannedKind::MitigationAddresses,
            score(format!(
                "How fully does '{}' address the cause '{cause}'?",
                m.description
            )),
        ));
        drafts.push((
            CHECK_MITIGATIONS,
            format!("{}/{}#class", fm.id, m.id),
            PlannedKind::MitigationClass { declared: m.kind },
            Question::Choice {
                instructions: json!(format!(
                    "Which class best describes the mitigation '{}' for the failure mode '{}'?",
                    m.description, fm.description
                )),
                criteria: BTreeMap::from([
                    (
                        "prevention".to_string(),
                        Some(json!("stops the failure mode from occurring")),
                    ),
                    (
                        "detection".to_string(),
                        Some(json!("reveals the failure mode when it occurs")),
                    ),
                    (
                        "fail_fast".to_string(),
                        Some(json!("bounds the effect by failing quickly and safely")),
                    ),
                ]),
            },
        ));
    }
}

fn noul(instructions: String) -> Question {
    Question::Noul {
        instructions: json!(instructions),
        criteria: None,
    }
}

fn score(instructions: String) -> Question {
    Question::Score {
        instructions: json!(instructions),
        criteria: SCORE_LEVELS.iter().map(|level| json!(level)).collect(),
    }
}

fn kind_label(kind: MitigationKind) -> &'static str {
    match kind {
        MitigationKind::Prevention => "prevention",
        MitigationKind::Detection => "detection",
        MitigationKind::FailFast => "fail_fast",
    }
}

/// Plan, send at most `max_questions` (clamped to [`HARD_MAX_QUESTIONS`]) in
/// one call, and report. Without a judge the result is `vet_unavailable`
/// with reason `no_key`; when nothing is to be sent the judge is not
/// consulted and the result is `ok`.
pub async fn run_vet(
    judge: Option<&dyn JudgmentModel>,
    subject: VetSubject,
    checks: &[&str],
    max_questions: usize,
) -> Value {
    run_vet_with(
        judge.ok_or_else(Unavailable::no_key),
        subject,
        checks,
        max_questions,
    )
    .await
}

/// [`run_vet`] with the reason the judge is missing, when it is.
pub async fn run_vet_with(
    judge: Result<&dyn JudgmentModel, Unavailable>,
    subject: VetSubject,
    checks: &[&str],
    max_questions: usize,
) -> Value {
    let state = subject_state(&subject);
    let mut planned = plan_questions(&subject, checks);
    let skipped: Vec<String> = planned
        .split_off(max_questions.min(HARD_MAX_QUESTIONS).min(planned.len()))
        .into_iter()
        .map(|p| p.id)
        .collect();
    let gate = subject.readiness_gate;
    if planned.is_empty() {
        return ok_report(None, None, gate, Vec::new(), skipped, 0);
    }
    let judge = match judge {
        Ok(judge) => judge,
        Err(unavailable) => return unavailable_report(&unavailable, gate),
    };
    let questions: BTreeMap<String, Question> = planned
        .iter()
        .map(|p| (p.id.clone(), p.question.clone()))
        .collect();
    let decisions = match judge.decide(state, questions).await {
        Ok(decisions) => decisions,
        Err(unavailable) => return unavailable_report(&unavailable, gate),
    };
    let mut findings = Vec::with_capacity(planned.len());
    for p in &planned {
        let Some(finding) = decisions
            .answers
            .get(&p.id)
            .and_then(|answer| interpret(p, answer))
        else {
            let bad = Unavailable {
                reason: UnavailableReason::BadResponse,
                detail: format!("no usable answer for question {} ({})", p.id, p.target),
            };
            return unavailable_report(&bad, gate);
        };
        findings.push(finding);
    }
    let sent = planned.len();
    ok_report(
        Some(decisions.model),
        decisions.cost,
        gate,
        findings,
        skipped,
        sent,
    )
}

fn ok_report(
    model: Option<String>,
    cost: Option<f64>,
    readiness_gate: Value,
    findings: Vec<Value>,
    skipped: Vec<String>,
    questions_sent: usize,
) -> Value {
    json!({
        "status": "ok",
        "model": model,
        "usage": { "cost": cost },
        "readiness_gate": readiness_gate,
        "findings": findings,
        "skipped": skipped,
        "questions_sent": questions_sent,
    })
}

/// The `vet_unavailable` report for `unavailable`, with the gate beside it.
pub fn unavailable_report(unavailable: &Unavailable, readiness_gate: Value) -> Value {
    json!({
        "status": "vet_unavailable",
        "reason": unavailable.reason,
        "detail": unavailable.detail,
        "readiness_gate": readiness_gate,
    })
}

/// A probability Jev could legitimately return: finite and in `[0, 1]`.
fn probability(value: f64) -> Option<f64> {
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}

/// The finding for one answer, or `None` when the answer has the wrong type
/// or an impossible value (the whole call is then `bad_response`).
fn interpret(planned: &Planned, answer: &Answer) -> Option<Value> {
    let mut finding = Map::new();
    finding.insert("check".into(), json!(planned.check));
    finding.insert("target".into(), json!(planned.target));
    finding.insert("question_hash".into(), json!(planned.id));
    let (p, threshold, flagged) = match (&planned.kind, answer) {
        (PlannedKind::ClaimedObservation, Answer::Noul { noul }) => {
            let p = probability(*noul)?;
            finding.insert("claimed".into(), json!(true));
            (p, CLAIMED_OBS_MIN, p < CLAIMED_OBS_MIN)
        }
        (PlannedKind::UnclaimedObservation, Answer::Noul { noul }) => {
            let p = probability(*noul)?;
            finding.insert("claimed".into(), json!(false));
            (p, UNCLAIMED_OBS_MAX, p > UNCLAIMED_OBS_MAX)
        }
        (PlannedKind::MitigationAddresses, Answer::Score { score, .. }) => {
            let p = score_probability(*score)?;
            (p, MITIGATION_ADDRESSES_MIN, p < MITIGATION_ADDRESSES_MIN)
        }
        (PlannedKind::Coverage, Answer::Score { score, .. }) => {
            let p = score_probability(*score)?;
            (p, COVERAGE_GAP_MIN, p >= COVERAGE_GAP_MIN)
        }
        (PlannedKind::Readiness, Answer::Score { score, .. }) => {
            let p = score_probability(*score)?;
            (p, READINESS_MIN, p < READINESS_MIN)
        }
        (PlannedKind::MitigationClass { declared }, Answer::Choice { probabilities, .. }) => {
            let Question::Choice { criteria, .. } = &planned.question else {
                return None;
            };
            let mut best: Option<(&String, f64)> = None;
            for (label, p) in probabilities {
                let p = probability(*p)?;
                if !criteria.contains_key(label) {
                    return None;
                }
                if best.is_none_or(|(_, top)| p > top) {
                    best = Some((label, p));
                }
            }
            let (choice, top) = best?;
            let declared = kind_label(*declared);
            let p = probabilities.get(declared).copied().unwrap_or(0.0);
            finding.insert("declared".into(), json!(declared));
            finding.insert("choice".into(), json!(choice));
            (
                p,
                MITIGATION_CLASS_CONFIDENCE,
                choice != declared && top >= MITIGATION_CLASS_CONFIDENCE,
            )
        }
        _ => return None,
    };
    let verdict = match (flagged, &planned.kind) {
        (false, _) => "ok",
        (true, PlannedKind::Readiness) => "concern",
        (true, _) => "flag",
    };
    finding.insert("probability".into(), json!(p));
    finding.insert("threshold".into(), json!(threshold));
    finding.insert("verdict".into(), json!(verdict));
    Some(Value::Object(finding))
}

/// A Score's probability, `score / 4`, for a score in `0..=4`.
fn score_probability(score: f64) -> Option<f64> {
    let top = (SCORE_LEVELS.len() - 1) as f64;
    (score.is_finite() && (0.0..=top).contains(&score)).then(|| score / top)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> VetSubject {
        let fms: Vec<AnalyzeInput> = serde_json::from_value(json!([{
            "id": "fm-1",
            "component": { "id": "c" },
            "description": "d",
            "domain": "runtime",
            "severity_observations": ["service_outage"],
            "mitigations": [{ "id": "m", "kind": "prevention", "description": "x" }]
        }]))
        .unwrap_or_default();
        subject_from_analysis(MatrixStrategy::default(), &fms).unwrap_or(VetSubject {
            strategy: MatrixStrategy::default(),
            failure_modes: Vec::new(),
            readiness_gate: Value::Null,
        })
    }

    fn targets(checks: &[&str]) -> Vec<String> {
        plan_questions(&subject(), checks)
            .into_iter()
            .map(|p| p.target)
            .collect()
    }

    #[test]
    fn top_level_claim_asks_no_unclaimed_severity_question() {
        assert_eq!(
            targets(&["observations"]),
            ["fm-1/service_outage", "fm-1/rare_edge_case"]
        );
    }

    #[test]
    fn mitigation_questions_are_score_then_class() {
        assert_eq!(targets(&["mitigations"]), ["fm-1/m", "fm-1/m#class"]);
    }

    #[test]
    fn hash_ignores_map_insertion_order() {
        let q = noul("q".to_string());
        assert_eq!(
            question_hash("c", "t", &q, &json!({ "a": 1, "b": 2 })),
            question_hash("c", "t", &q, &json!({ "b": 2, "a": 1 }))
        );
    }

    #[test]
    fn hash_matches_the_canonical_document() {
        let (q, state) = (
            noul("q".to_string()),
            json!({ "b": [1, { "d": 2, "c": 3 }], "a": "x" }),
        );
        let document = canonical(&json!({
            "check": "c", "target": "t", "question": q, "state": state,
        }));
        let expected: String = Sha256::digest(serde_json::to_vec(&document).unwrap_or_default())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(question_hash("c", "t", &q, &state), expected);
    }

    #[test]
    fn score_of_two_is_one_half() {
        assert_eq!(score_probability(2.0), Some(0.5));
    }
}
