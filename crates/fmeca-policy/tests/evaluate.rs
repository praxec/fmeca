//! The deterministic evaluator (spec §4): confidence ladder, bands, fallback,
//! dispositions, actions, rollups and sign-off.

use fmeca::{
    Disposition, Domain, EvidenceBasis, EvidenceKind, MatrixStrategy, MitigationKind,
    MitigationStatus, ResolutionEvidence,
};
use fmeca_policy::{
    Accepted, Confidence, Date, DispositionKind, DispositionStatus, KernelGate, LoadedPolicy,
    PolicyError, PolicySource, Subject, SubjectFm, SubjectMitigation, Suggestions, evaluate,
    evidence_confidence, parse_date, parse_policy, policy_hash, residual_confidence,
    subject_from_state, validate,
};

/// Three bands (`none` 0, `act` 1, `review` 2); `act` needs C1 and `review`
/// C2; both review every 90 days; sign-off at `act`. Two actions: an
/// `eliminate` action on `review` (required for class `critical`) and a
/// `detect` action on `act` and above below C3.
///
/// The 3×3 matrix (rows severity 1..3, columns probability 1..3):
///
/// ```text
/// none   none   act
/// none   act    review
/// act    review review
/// ```
const TEST_POLICY: &str = r#"
name = "test"
description = "Evaluator fixture"
class = "standard"
schema = 1

[[bands]]
id = "none"
label = "No action"
rank = 0
dispositions = ["close", "accept"]

[[bands]]
id = "act"
label = "Act"
rank = 1
dispositions = ["mitigate", "watch", "accept"]
no_action_needs_rationale = true
min_confidence = "C1"
acceptance = "tech_lead"
review_interval = "90d"

[[bands]]
id = "review"
label = "Review"
rank = 2
dispositions = ["mitigate", "research", "elevate", "accept"]
no_action_needs_rationale = true
min_confidence = "C2"
acceptance = "eng_manager"
concurrence = ["tech_lead"]
review_interval = "90d"

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["none", "none",   "act"],
  ["none", "act",    "review"],
  ["act",  "review", "review"],
]

[signoff]
max_residual_band = "act"

[[actions]]
id = "remove-path"
text = "Remove the failure path by design"
tags = ["design"]
precedence = "eliminate"
applies_when = { band_at_least = "review" }
required_when = { class_in = ["critical"] }

[[actions]]
id = "add-alarm"
text = "Alarm on the leading indicator"
tags = ["ops", "test"]
precedence = "detect"
applies_when = { band_at_least = "act", confidence_below = "C3" }
"#;

/// Parse, validate and hash a policy document.
fn policy_from(text: &str) -> LoadedPolicy {
    let policy = parse_policy(text).unwrap();
    validate(&policy).unwrap();
    LoadedPolicy {
        hash: policy_hash(&policy),
        policy,
        source: PolicySource::Project,
    }
}

fn test_policy() -> LoadedPolicy {
    policy_from(TEST_POLICY)
}

fn as_of() -> Date {
    parse_date("2026-10-12").unwrap()
}

fn evidence(kind: EvidenceKind, basis: Option<EvidenceBasis>) -> ResolutionEvidence {
    ResolutionEvidence {
        kind,
        reference: "ref".into(),
        basis,
    }
}

/// An open, scored, unmitigated runtime failure mode.
fn fm(id: &str, severity: u8, probability: u8) -> SubjectFm {
    SubjectFm {
        id: id.into(),
        domain: Domain::Runtime,
        single_point: false,
        inherent: Some((severity, probability)),
        residual: None,
        response_class: None,
        disposition: Disposition::Open,
        disposition_rationale: None,
        accepted: None,
        mitigations: vec![],
    }
}

/// An active (implemented) mitigation achieving `residual`.
fn mitigation(
    id: &str,
    kind: MitigationKind,
    residual: (u8, u8),
    evidence: Vec<ResolutionEvidence>,
) -> SubjectMitigation {
    SubjectMitigation {
        id: id.into(),
        kind,
        status: MitigationStatus::Implemented,
        residual: Some(residual),
        evidence,
    }
}

/// `fm` with one mitigation; the failure mode's residual is the mitigation's.
fn mitigated(mut f: SubjectFm, m: SubjectMitigation) -> SubjectFm {
    f.residual = m.residual;
    f.mitigations.push(m);
    f
}

fn accepted(mut f: SubjectFm, at: &str) -> SubjectFm {
    f.disposition = Disposition::Accepted;
    f.accepted = Some(Accepted {
        approver: "alice".into(),
        at: Some(at.into()),
        rationale: "known and bounded".into(),
    });
    f
}

fn subject(failure_modes: Vec<SubjectFm>) -> Subject {
    Subject {
        strategy: MatrixStrategy::Qualitative3x3,
        level: None,
        failure_modes,
        kernel_gate: KernelGate {
            ready: true,
            blockers: vec![],
        },
    }
}

fn eval(failure_modes: Vec<SubjectFm>) -> Suggestions {
    evaluate(&subject(failure_modes), &test_policy(), as_of()).unwrap()
}

fn confidence_of(evidence: Vec<ResolutionEvidence>) -> Confidence {
    residual_confidence(&mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::Prevention, (1, 1), evidence),
    ))
}

// --- confidence ladder -------------------------------------------------------

#[test]
fn note_only_evidence_is_c0() {
    assert_eq!(
        confidence_of(vec![evidence(EvidenceKind::Note, None)]),
        Confidence::C0
    );
}

#[test]
fn url_with_design_basis_is_c1() {
    assert_eq!(
        evidence_confidence(&evidence(EvidenceKind::Url, Some(EvidenceBasis::Design))),
        Confidence::C1
    );
}

#[test]
fn file_without_basis_is_c1() {
    assert_eq!(
        evidence_confidence(&evidence(EvidenceKind::File, None)),
        Confidence::C1
    );
}

#[test]
fn commit_is_c2() {
    assert_eq!(
        evidence_confidence(&evidence(EvidenceKind::Commit, None)),
        Confidence::C2
    );
}

#[test]
fn inspection_is_c2() {
    assert_eq!(
        evidence_confidence(&evidence(
            EvidenceKind::File,
            Some(EvidenceBasis::Inspection)
        )),
        Confidence::C2
    );
}

#[test]
fn test_kind_is_c3() {
    assert_eq!(
        evidence_confidence(&evidence(EvidenceKind::Test, None)),
        Confidence::C3
    );
}

#[test]
fn operational_use_is_c4() {
    assert_eq!(
        evidence_confidence(&evidence(
            EvidenceKind::Url,
            Some(EvidenceBasis::OperationalUse)
        )),
        Confidence::C4
    );
}

#[test]
fn best_evidence_wins() {
    assert_eq!(
        confidence_of(vec![
            evidence(EvidenceKind::Note, None),
            evidence(EvidenceKind::Test, None),
            evidence(EvidenceKind::Commit, None),
        ]),
        Confidence::C3
    );
}

#[test]
fn no_mitigation_is_c0() {
    assert_eq!(residual_confidence(&fm("fm1", 3, 3)), Confidence::C0);
}

// --- bands and fallback ------------------------------------------------------

#[test]
fn inherent_band_from_matrix() {
    let s = eval(vec![fm("fm1", 2, 3)]);
    assert_eq!(s.failure_modes[0].inherent_band.as_deref(), Some("review"));
}

/// A projected session: `fm1` (high × high) with an abandoned mitigation that
/// would bring it to `none` (low × low) and an implemented one that brings it
/// to `act` (low × high).
const ABANDONED_LOG: &str = r#"
{"type":"session_opened","session_id":"s1","matrix_strategy":"qualitative3x3"}
{"type":"failure_mode_added","failure_mode":{"id":"fm1","session_id":"s1","component":{"id":"c1"},"description":"d","cause":"c","effect":"e","severity_observations":["data_loss"],"probability_observations":["happens_in_normal_use"],"domain":"runtime","source":{"turn_id":"t1"}}}
{"type":"mitigation_added","mitigation":{"id":"m1","session_id":"s1","failure_mode_id":"fm1","kind":"prevention","description":"gone","residual_severity_observations":["cosmetic"],"residual_probability_observations":["rare_edge_case"],"source":{"turn_id":"t1"}}}
{"type":"mitigation_added","mitigation":{"id":"m2","session_id":"s1","failure_mode_id":"fm1","kind":"detection","description":"kept","residual_severity_observations":["cosmetic"],"residual_probability_observations":["happens_in_normal_use"],"source":{"turn_id":"t1"}}}
{"type":"mitigation_status_changed","change":{"session_id":"s1","mitigation_id":"m1","status":"abandoned"}}
{"type":"mitigation_status_changed","change":{"session_id":"s1","mitigation_id":"m2","status":"implemented","evidence":[{"kind":"test","reference":"t"}]}}
"#;

#[test]
fn residual_band_ignores_abandoned() {
    let events: Vec<fmeca::Event> = ABANDONED_LOG
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let state = fmeca::projection::project(&events);
    let s = evaluate(&subject_from_state(&state), &test_policy(), as_of()).unwrap();
    assert_eq!(s.failure_modes[0].residual_band.as_deref(), Some("act"));
}

/// Review Focus 3: a mitigation declared with no evidence is C0, below the
/// residual band's `min_confidence`, so the inherent band applies.
#[test]
fn declared_only_mitigation_falls_back_to_inherent_band() {
    let s = eval(vec![mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::Prevention, (1, 3), vec![]),
    )]);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("review"));
}

/// `review` → `act` leaves `review`, so the threshold is `review`'s C2 (the
/// first gated band left, in either evidence mode), not `act`'s C1; a
/// commit (C2) is exactly at it.
#[test]
fn fallback_absent_at_threshold() {
    let s = eval(vec![mitigated(
        fm("fm1", 3, 3),
        mitigation(
            "m1",
            MitigationKind::Prevention,
            (1, 3),
            vec![evidence(EvidenceKind::Commit, None)],
        ),
    )]);
    assert!(s.failure_modes[0].fallback.is_none());
}

#[test]
fn leaving_a_band_needs_that_bands_confidence() {
    // `review` → `act` at C1: `act`'s own C1 is met, `review`'s C2 is not.
    let s = eval(vec![mitigated(
        fm("fm1", 3, 3),
        mitigation(
            "m1",
            MitigationKind::Prevention,
            (1, 3),
            vec![evidence(EvidenceKind::File, None)],
        ),
    )]);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("review"));
}

// --- dispositions ------------------------------------------------------------

#[test]
fn open_low_confidence_suggests_research() {
    // Residual `act` at C0 falls back to `review`, which allows `research`.
    let s = eval(vec![mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::Prevention, (1, 3), vec![]),
    )]);
    assert_eq!(
        s.failure_modes[0].status,
        DispositionStatus::Open {
            suggested: DispositionKind::Research
        }
    );
}

#[test]
fn open_suggests_mitigate_when_allowed() {
    let s = eval(vec![fm("fm1", 3, 3)]);
    assert_eq!(
        s.failure_modes[0].status,
        DispositionStatus::Open {
            suggested: DispositionKind::Mitigate
        }
    );
}

#[test]
fn unscored_suggests_research() {
    let mut f = fm("fm1", 1, 1);
    f.inherent = None;
    let s = eval(vec![f]);
    assert_eq!(
        s.failure_modes[0].status,
        DispositionStatus::Unscored {
            suggested: DispositionKind::Research
        }
    );
}

fn review_due(s: &Suggestions) -> Option<bool> {
    match &s.failure_modes[0].status {
        DispositionStatus::Accepted { review_due, .. } => Some(*review_due),
        _ => None,
    }
}

#[test]
fn accepted_review_due_after_interval() {
    // 2026-07-13 is 91 days before 2026-10-12.
    let s = eval(vec![accepted(fm("fm1", 3, 3), "2026-07-13T09:00:00Z")]);
    assert_eq!(review_due(&s), Some(true));
}

#[test]
fn accepted_review_not_due_within_interval() {
    // 2026-07-14 is exactly 90 days before 2026-10-12.
    let s = eval(vec![accepted(fm("fm1", 3, 3), "2026-07-14T09:00:00Z")]);
    assert_eq!(review_due(&s), Some(false));
}

/// Review Focus 4: an accepted risk above the sign-off band whose review is
/// overdue fails sign-off when `accepted_requires_review_current`.
#[test]
fn overdue_accepted_risk_fails_signoff() {
    let policy = policy_from(&TEST_POLICY.replace(
        "max_residual_band = \"act\"",
        "max_residual_band = \"act\"\naccepted_requires_review_current = true",
    ));
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-01-01T00:00:00Z")]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert!(!s.signoff.pass);
}

#[test]
fn rationale_needed_without_mitigation() {
    let s = eval(vec![fm("fm1", 2, 2)]);
    assert!(s.failure_modes[0].rationale_needed);
}

// --- actions -----------------------------------------------------------------

fn action_ids(s: &Suggestions) -> Vec<&str> {
    s.failure_modes[0]
        .actions
        .iter()
        .map(|a| a.id.as_str())
        .collect()
}

#[test]
fn actions_sorted_by_precedence() {
    let s = eval(vec![fm("fm1", 3, 3)]);
    assert_eq!(action_ids(&s), vec!["remove-path", "add-alarm"]);
}

#[test]
fn required_when_class_marks_required() {
    let policy = policy_from(&TEST_POLICY.replace("class = \"standard\"", "class = \"critical\""));
    let s = evaluate(&subject(vec![fm("fm1", 3, 3)]), &policy, as_of()).unwrap();
    assert!(s.failure_modes[0].actions[0].required);
}

#[test]
fn covered_by_matching_mitigation_kind() {
    // A detection mitigation that leaves the residual in `review` covers the
    // `detect` action.
    let s = eval(vec![mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::Detection, (3, 2), vec![]),
    )]);
    assert_eq!(
        s.failure_modes[0].actions[1].covered_by,
        vec!["m1".to_string()]
    );
}

#[test]
fn eliminate_never_auto_covered() {
    let mut f = mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::Prevention, (3, 2), vec![]),
    );
    f.mitigations
        .push(mitigation("m2", MitigationKind::Detection, (3, 3), vec![]));
    f.mitigations
        .push(mitigation("m3", MitigationKind::FailFast, (3, 3), vec![]));
    let s = eval(vec![f]);
    assert!(s.failure_modes[0].actions[0].covered_by.is_empty());
}

// --- analysis-level outputs ----------------------------------------------------

#[test]
fn excluded_listed_separately() {
    let mut f = fm("fm2", 3, 3);
    f.disposition = Disposition::Excluded;
    f.disposition_rationale = Some("not credible".into());
    let s = eval(vec![fm("fm1", 1, 1), f]);
    assert_eq!(
        (
            s.failure_modes.len(),
            s.excluded[0].id.as_str(),
            s.excluded[0].rationale.as_str()
        ),
        (1, "fm2", "not credible")
    );
}

#[test]
fn signoff_passes_within_max_band() {
    let s = eval(vec![fm("fm1", 1, 1), fm("fm2", 1, 3)]);
    assert!(s.signoff.pass);
}

#[test]
fn signoff_fails_above_max_band_with_reason() {
    let s = eval(vec![fm("fm1", 1, 1), fm("fm2", 3, 3)]);
    assert_eq!(
        s.signoff.reasons,
        vec!["fm2: effective band `review` ranks above max_residual_band `act`".to_string()]
    );
}

#[test]
fn required_uncovered_fails_signoff() {
    let policy = policy_from(&TEST_POLICY.replace("class = \"standard\"", "class = \"critical\""));
    // fm1 is open, so nothing covers the required eliminate action.
    let subject = subject(vec![fm("fm1", 3, 3)]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert!(
        s.signoff
            .reasons
            .contains(&"fm1: required action `remove-path` is not covered".to_string())
    );
}

#[test]
fn kernel_blocker_fails_signoff_when_included() {
    let mut subject = subject(vec![fm("fm1", 1, 1)]);
    subject.kernel_gate = KernelGate {
        ready: false,
        blockers: vec!["CLARIFY fm1".into()],
    };
    let s = evaluate(&subject, &test_policy(), as_of()).unwrap();
    assert_eq!(
        (s.signoff.pass, s.signoff.reasons),
        (false, vec!["kernel: CLARIFY fm1".to_string()])
    );
}

#[test]
fn kernel_blocker_ignored_when_excluded() {
    let policy = policy_from(&TEST_POLICY.replace(
        "max_residual_band = \"act\"",
        "max_residual_band = \"act\"\ninclude_kernel_gate = false",
    ));
    let mut subject = subject(vec![fm("fm1", 1, 1)]);
    subject.kernel_gate = KernelGate {
        ready: false,
        blockers: vec!["CLARIFY fm1".into()],
    };
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert!(s.signoff.pass);
}

#[test]
fn same_input_gives_identical_json() {
    let failure_modes = vec![
        accepted(fm("fm3", 3, 3), "2026-01-01T00:00:00Z"),
        mitigated(
            fm("fm1", 3, 3),
            mitigation(
                "m1",
                MitigationKind::Detection,
                (1, 3),
                vec![evidence(EvidenceKind::Commit, None)],
            ),
        ),
        fm("fm2", 2, 2),
    ];
    let mut reversed = failure_modes.clone();
    reversed.reverse();
    let first = serde_json::to_string(&eval(failure_modes)).unwrap();
    let second = serde_json::to_string(&eval(reversed)).unwrap();
    assert_eq!(first, second);
}

/// A `confidence-view`-style policy: three bands and a confidence matrix whose
/// rows are the standard band rank and whose columns are C4…C0.
const CONFIDENCE_POLICY: &str = r#"
name = "confidence-fixture"
description = "Effective band × confidence"
class = "standard"
schema = 1

[[bands]]
id = "none"
label = "None"
rank = 0
dispositions = ["close", "accept"]

[[bands]]
id = "report"
label = "Report"
rank = 1
dispositions = ["mitigate", "accept"]

[[bands]]
id = "remediate"
label = "Remediate"
rank = 2
dispositions = ["mitigate", "accept"]

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["none", "none",      "report"],
  ["none", "report",    "remediate"],
  ["report", "remediate", "remediate"],
]

[matrices.confidence]
derived_from = "standard-bands"
cells = [
  ["none",   "none",   "none",      "none",      "report"],
  ["none",   "none",   "report",    "report",    "remediate"],
  ["report", "report", "remediate", "remediate", "remediate"],
]

[signoff]
max_residual_band = "report"
"#;

#[test]
fn confidence_matrix_maps_band_and_confidence() {
    // Standard rank 1 (`report`) at C0 (no mitigation) → `remediate`.
    let s = evaluate(
        &subject(vec![fm("fm1", 2, 2)]),
        &policy_from(CONFIDENCE_POLICY),
        as_of(),
    )
    .unwrap();
    assert_eq!(
        s.failure_modes[0].effective_band.as_deref(),
        Some("remediate")
    );
}

#[test]
fn strategy_without_matrix_is_mismatch() {
    let mut subject = subject(vec![fm("fm1", 3, 3)]);
    subject.strategy = MatrixStrategy::Nasa8004_5x5;
    assert!(matches!(
        evaluate(&subject, &test_policy(), as_of()),
        Err(PolicyError::StrategyMismatch(_))
    ));
}

// --- inputs and grouping -------------------------------------------------------

#[test]
fn analysis_subject_carries_kernel_blockers() {
    // An unscored failure mode is a kernel CLARIFY blocker.
    let input: Vec<fmeca::AnalyzeInput> = serde_json::from_str(
        r#"[{"id":"fm1","component":{"id":"c1"},"description":"d","domain":"runtime"}]"#,
    )
    .unwrap();
    let subject =
        fmeca_policy::subject_from_analysis(MatrixStrategy::Qualitative3x3, &input).unwrap();
    assert!(!subject.kernel_gate.blockers.is_empty());
}

#[test]
fn rfc3339_timestamp_echoes_its_date() {
    assert_eq!(
        parse_date("2024-02-29T23:59:59Z").unwrap().to_string(),
        "2024-02-29"
    );
}

#[test]
fn impossible_date_is_invalid() {
    assert!(matches!(
        parse_date("2026-02-29"),
        Err(PolicyError::Invalid { .. })
    ));
}

#[test]
fn group_by_band_lists_failure_modes() {
    let s = eval(vec![fm("fm2", 3, 3), fm("fm1", 3, 3), fm("fm3", 1, 1)]);
    assert_eq!(
        fmeca_policy::group(&s, fmeca_policy::GroupBy::Band)["review"],
        vec!["fm1".to_string(), "fm2".to_string()]
    );
}

// --- fix round 1 -----------------------------------------------------------------

#[test]
fn note_with_test_basis_is_still_c0() {
    assert_eq!(
        evidence_confidence(&evidence(EvidenceKind::Note, Some(EvidenceBasis::Test))),
        Confidence::C0
    );
}

#[test]
fn failure_history_basis_is_c4() {
    assert_eq!(
        evidence_confidence(&evidence(
            EvidenceKind::File,
            Some(EvidenceBasis::FailureHistory)
        )),
        Confidence::C4
    );
}

#[test]
fn pull_request_is_c2() {
    assert_eq!(
        evidence_confidence(&evidence(EvidenceKind::PullRequest, None)),
        Confidence::C2
    );
}

#[test]
fn low_confidence_never_lowers_band() {
    // Inherent `act` (1, 3); the recorded residual (3, 3) is worse, `review`,
    // at C0 — below `review`'s C2, but falling back would lower the band.
    let s = eval(vec![mitigated(
        fm("fm1", 1, 3),
        mitigation("m1", MitigationKind::Prevention, (3, 3), vec![]),
    )]);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("review"));
}

#[test]
fn planned_mitigation_does_not_cover() {
    let mut m = mitigation("m1", MitigationKind::Detection, (3, 2), vec![]);
    m.status = MitigationStatus::Planned;
    let s = eval(vec![mitigated(fm("fm1", 3, 3), m)]);
    assert!(s.failure_modes[0].actions[1].covered_by.is_empty());
}

#[test]
fn contain_is_covered_by_fail_fast() {
    let policy =
        policy_from(&TEST_POLICY.replace("precedence = \"detect\"", "precedence = \"contain\""));
    let subject = subject(vec![mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::FailFast, (3, 2), vec![]),
    )]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert_eq!(
        s.failure_modes[0].actions[1].covered_by,
        vec!["m1".to_string()]
    );
}

fn unscored(id: &str) -> SubjectFm {
    let mut f = mitigated(
        fm(id, 1, 1),
        mitigation(
            "m1",
            MitigationKind::Prevention,
            (1, 1),
            vec![evidence(EvidenceKind::Test, None)],
        ),
    );
    f.inherent = None;
    f
}

#[test]
fn unscored_is_c0() {
    assert_eq!(
        eval(vec![unscored("fm1")]).failure_modes[0].confidence,
        Confidence::C0
    );
}

#[test]
fn unscored_fails_signoff_without_kernel_gate() {
    let policy = policy_from(&TEST_POLICY.replace(
        "max_residual_band = \"act\"",
        "max_residual_band = \"act\"\ninclude_kernel_gate = false",
    ));
    let s = evaluate(&subject(vec![unscored("fm1")]), &policy, as_of()).unwrap();
    assert_eq!(s.signoff.reasons, vec!["fm1: unscored".to_string()]);
}

#[test]
fn unscored_reason_omitted_when_unscored_fails_is_false() {
    let policy = policy_from(&TEST_POLICY.replace(
        "max_residual_band = \"act\"",
        "max_residual_band = \"act\"\ninclude_kernel_gate = false\nunscored_fails = false",
    ));
    let s = evaluate(&subject(vec![unscored("fm1")]), &policy, as_of()).unwrap();
    assert!(s.signoff.pass, "{:?}", s.signoff.reasons);
}

#[test]
fn open_unscored_still_fails_through_kernel_clarify_when_unscored_fails_is_false() {
    let policy = policy_from(&TEST_POLICY.replace(
        "max_residual_band = \"act\"",
        "max_residual_band = \"act\"\nunscored_fails = false",
    ));
    let input: Vec<fmeca::AnalyzeInput> = serde_json::from_str(
        r#"[{"id":"fm1","component":{"id":"c1"},"description":"d","domain":"runtime"}]"#,
    )
    .unwrap();
    let subject =
        fmeca_policy::subject_from_analysis(MatrixStrategy::Qualitative3x3, &input).unwrap();
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert!(
        s.signoff
            .reasons
            .iter()
            .all(|r| r.starts_with("kernel: CLARIFY"))
            && !s.signoff.pass,
        "{:?}",
        s.signoff.reasons
    );
}

#[test]
fn parse_date_rejects_trailing_junk() {
    assert!(matches!(
        parse_date("2026-10-12Tjunk"),
        Err(PolicyError::Invalid { .. })
    ));
}

#[test]
fn overdue_reason_only_when_acceptance_counts() {
    let policy = policy_from(&TEST_POLICY.replace(
        "max_residual_band = \"act\"",
        "max_residual_band = \"act\"\naccepted_counts_as_pass = false\naccepted_requires_review_current = true",
    ));
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-01-01T00:00:00Z")]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert_eq!(
        s.signoff.reasons,
        vec!["fm1: effective band `review` ranks above max_residual_band `act`".to_string()]
    );
}

// --- acceptance must be allowed by the band ----------------------------------

#[test]
fn acceptance_not_allowed_in_band_fails_signoff() {
    let policy = policy_from(&TEST_POLICY.replace(
        r#"dispositions = ["mitigate", "research", "elevate", "accept"]"#,
        r#"dispositions = ["mitigate", "research", "elevate"]"#,
    ));
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-10-01T00:00:00Z")]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert_eq!(
        s.signoff.reasons,
        vec!["fm1: accepted in band 'review' which does not allow acceptance".to_string()]
    );
}

#[test]
fn acceptance_allowed_in_band_passes() {
    let s = eval(vec![accepted(fm("fm1", 3, 3), "2026-10-01T00:00:00Z")]);
    assert!(s.signoff.pass, "{:?}", s.signoff.reasons);
}

// --- rank_at_least and mitigation_kinds_present ------------------------------

/// `TEST_POLICY` plus one action with the given `applies_when`.
fn policy_with_action(applies_when: &str) -> LoadedPolicy {
    policy_from(&format!(
        "{TEST_POLICY}\n[[actions]]\nid = \"probe\"\ntext = \"Probe\"\ntags = [\"test\"]\nprecedence = \"procedure\"\napplies_when = {applies_when}\n"
    ))
}

/// The ids of the failure modes that were suggested the `probe` action.
fn probed(policy: &LoadedPolicy, failure_modes: Vec<SubjectFm>) -> Vec<String> {
    evaluate(&subject(failure_modes), policy, as_of())
        .unwrap()
        .failure_modes
        .into_iter()
        .filter(|f| f.actions.iter().any(|a| a.id == "probe"))
        .map(|f| f.id)
        .collect()
}

#[test]
fn rank_at_least_matches_effective_band_rank() {
    let policy = policy_with_action("{ rank_at_least = 1 }");
    // fm1 is `none` (rank 0), fm2 `act` (rank 1), fm3 `review` (rank 2).
    let ids = probed(
        &policy,
        vec![fm("fm1", 1, 1), fm("fm2", 2, 2), fm("fm3", 3, 3)],
    );
    assert_eq!(ids, vec!["fm2".to_string(), "fm3".to_string()]);
}

#[test]
fn rank_at_least_never_matches_unscored() {
    let policy = policy_with_action("{ rank_at_least = 0 }");
    assert!(probed(&policy, vec![unscored("fm1")]).is_empty());
}

#[test]
fn rank_at_least_uses_the_fallback_band() {
    // Declared-only mitigation (C0) on a `review` inherent risk: the residual
    // `act` (min C1) falls back to `review`, so a rank-2 gate matches.
    let policy = policy_with_action("{ rank_at_least = 2 }");
    let f = mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::Prevention, (2, 2), vec![]),
    );
    assert_eq!(probed(&policy, vec![f]), vec!["fm1".to_string()]);
}

#[test]
fn mitigation_kinds_present_needs_an_active_mitigation_of_the_kind() {
    let policy = policy_with_action(r#"{ mitigation_kinds_present = ["fail_fast"] }"#);
    let with_fail_fast = mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::FailFast, (2, 2), vec![]),
    );
    let with_prevention = mitigated(
        fm("fm2", 3, 3),
        mitigation("m2", MitigationKind::Prevention, (2, 2), vec![]),
    );
    let ids = probed(
        &policy,
        vec![with_fail_fast, with_prevention, fm("fm3", 3, 3)],
    );
    assert_eq!(ids, vec!["fm1".to_string()]);
}

// --- acceptance satisfies an uncoverable required action ---------------------

fn critical_policy() -> LoadedPolicy {
    policy_from(&TEST_POLICY.replace("class = \"standard\"", "class = \"critical\""))
}

#[test]
fn accepted_risk_satisfies_uncoverable_required_action() {
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-10-01T00:00:00Z")]);
    let s = evaluate(&subject, &critical_policy(), as_of()).unwrap();
    assert_eq!(
        s.failure_modes[0].actions[0].covered_by,
        vec![fmeca_policy::ACCEPTANCE.to_string()]
    );
}

#[test]
fn unaccepted_risk_leaves_uncoverable_required_action_uncovered() {
    let subject = subject(vec![fm("fm1", 3, 3)]);
    let s = evaluate(&subject, &critical_policy(), as_of()).unwrap();
    assert_eq!(
        s.rollups.required_uncovered,
        vec![("fm1".to_string(), "remove-path".to_string())]
    );
}

#[test]
fn acceptance_without_rationale_does_not_satisfy_required_action() {
    let mut f = accepted(fm("fm1", 3, 3), "2026-10-01T00:00:00Z");
    if let Some(a) = f.accepted.as_mut() {
        a.rationale = "  ".into();
    }
    let s = evaluate(&subject(vec![f]), &critical_policy(), as_of()).unwrap();
    assert!(s.failure_modes[0].actions[0].covered_by.is_empty());
}

#[test]
fn planned_fail_fast_does_not_count_as_present() {
    let policy = policy_with_action(r#"{ mitigation_kinds_present = ["fail_fast"] }"#);
    let mut m = mitigation("m1", MitigationKind::FailFast, (2, 2), vec![]);
    m.status = MitigationStatus::Planned;
    assert!(probed(&policy, vec![mitigated(fm("fm1", 3, 3), m)]).is_empty());
}

#[test]
fn planned_prevention_does_not_count_against_missing() {
    let policy = policy_with_action(r#"{ mitigation_kinds_missing = ["prevention"] }"#);
    let mut m = mitigation("m1", MitigationKind::Prevention, (2, 2), vec![]);
    m.status = MitigationStatus::Planned;
    assert_eq!(
        probed(&policy, vec![mitigated(fm("fm1", 3, 3), m)]),
        vec!["fm1".to_string()]
    );
}

#[test]
fn mitigation_kinds_present_names_each_kind_once() {
    let policy = policy_with_action(r#"{ mitigation_kinds_present = ["fail_fast"] }"#);
    let mut f = mitigated(
        fm("fm1", 3, 3),
        mitigation("m1", MitigationKind::FailFast, (2, 2), vec![]),
    );
    f.mitigations
        .push(mitigation("m2", MitigationKind::FailFast, (2, 2), vec![]));
    let s = evaluate(&subject(vec![f]), &policy, as_of()).unwrap();
    let why = s.failure_modes[0]
        .actions
        .iter()
        .find(|a| a.id == "probe")
        .map(|a| a.why.clone());
    assert_eq!(
        why,
        Some(vec!["fail_fast mitigation implemented".to_string()])
    );
}

#[test]
fn acceptance_in_disallowing_band_leaves_required_action_uncovered() {
    let policy = policy_from(
        &TEST_POLICY
            .replace("class = \"standard\"", "class = \"critical\"")
            .replace(
                r#"dispositions = ["mitigate", "research", "elevate", "accept"]"#,
                r#"dispositions = ["mitigate", "research", "elevate"]"#,
            ),
    );
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-10-01T00:00:00Z")]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert_eq!(
        s.rollups.required_uncovered,
        vec![("fm1".to_string(), "remove-path".to_string())]
    );
}

#[test]
fn overdue_acceptance_does_not_cover_required_action_when_review_must_be_current() {
    let policy = policy_from(
        &TEST_POLICY
            .replace("class = \"standard\"", "class = \"critical\"")
            .replace(
                "max_residual_band = \"act\"",
                "max_residual_band = \"act\"\naccepted_requires_review_current = true",
            ),
    );
    // Accepted 2026-01-01; `review` reviews every 90 days, so overdue.
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-01-01T00:00:00Z")]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert!(s.failure_modes[0].actions[0].covered_by.is_empty());
}

#[test]
fn acceptance_does_not_cover_required_action_when_acceptance_does_not_count() {
    let policy = policy_from(
        &TEST_POLICY
            .replace("class = \"standard\"", "class = \"critical\"")
            .replace(
                "max_residual_band = \"act\"",
                "max_residual_band = \"act\"\naccepted_counts_as_pass = false",
            ),
    );
    let subject = subject(vec![accepted(fm("fm1", 3, 3), "2026-10-01T00:00:00Z")]);
    let s = evaluate(&subject, &policy, as_of()).unwrap();
    assert!(s.failure_modes[0].actions[0].covered_by.is_empty());
}
