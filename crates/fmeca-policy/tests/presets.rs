//! Built-in presets, the shared action catalog, golden outputs and
//! `docs/policies.md` (spec §6, plan Task 3).
//!
//! Every test makes one behavioural assertion. Goldens are compared
//! byte-for-byte; regenerate them only with `FMECA_BLESS=1`.

use std::collections::BTreeSet;
use std::path::Path;

use fmeca::{
    AnalysisLevel, AnalyzeInput, AnalyzeMitigation, Disposition, Domain, EntityRef, MatrixStrategy,
    MitigationKind, MitigationStatus, ResolutionEvidence,
};
use fmeca_policy::presets::{PRESET_NAMES, builtin_actions, preset, resolve};
use fmeca_policy::{
    Accepted, Conditions, Date, KernelGate, LoadedPolicy, PolicySource, STANDARD_BAND_COUNT,
    Subject, SubjectFm, SubjectMitigation, evaluate, parse_date, parse_policy, policy_hash,
    subject_from_analysis, subject_from_state, validate,
};

fn as_of() -> Date {
    parse_date("2026-10-12").unwrap()
}

// --- presets -----------------------------------------------------------------

#[test]
fn all_presets_validate() {
    let all_ok = PRESET_NAMES
        .iter()
        .all(|name| preset(name).is_some_and(|lp| validate(&lp.policy).is_ok()));
    assert!(all_ok, "every built-in preset must parse and validate");
}

#[test]
fn standard_preset_matches_standard_band_count() {
    let standard = preset("standard").expect("standard preset");
    let mut ranks: Vec<u8> = standard.policy.bands.iter().map(|b| b.rank).collect();
    ranks.sort_unstable();
    assert_eq!(ranks, (0..STANDARD_BAND_COUNT as u8).collect::<Vec<_>>());
}

#[test]
fn every_preset_sets_include_kernel_gate() {
    let all = PRESET_NAMES
        .iter()
        .all(|name| preset(name).is_some_and(|lp| lp.policy.signoff.include_kernel_gate));
    assert!(all, "every preset must include the kernel gate");
}

#[test]
fn resolve_returns_a_preset_by_name() {
    assert!(resolve("standard", None).is_ok());
}

#[test]
fn resolve_unknown_name_without_project_dir_is_not_found() {
    assert!(resolve("no-such-policy", None).is_err());
}

// --- built-in action catalog -------------------------------------------------

#[test]
fn builtin_actions_have_unique_ids() {
    let actions = builtin_actions();
    let unique: BTreeSet<&str> = actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(unique.len(), actions.len());
}

#[test]
fn every_action_tag_is_known() {
    const KNOWN: [&str; 7] = [
        "design", "review", "test", "ops", "process", "docs", "security",
    ];
    let actions = builtin_actions();
    let unknown: Vec<&str> = actions
        .iter()
        .flat_map(|a| a.tags.iter().map(String::as_str))
        .filter(|tag| !KNOWN.contains(tag))
        .collect();
    assert!(unknown.is_empty(), "unknown action tags: {unknown:?}");
}

#[test]
fn builtin_catalog_has_at_least_24_actions() {
    assert!(builtin_actions().len() >= 24);
}

#[test]
fn every_builtin_action_declares_applies_when() {
    let actions = builtin_actions();
    let missing: Vec<&str> = actions
        .iter()
        .filter(|a| a.applies_when == Conditions::default())
        .map(|a| a.id.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "built-in actions without applies_when: {missing:?}"
    );
}

#[test]
fn preset_merges_builtin_catalog_when_requested() {
    let standard = preset("standard").expect("standard preset");
    let merged = builtin_actions()
        .iter()
        .all(|b| standard.policy.actions.iter().any(|a| a.id == b.id));
    assert!(merged, "standard must merge the built-in catalog");
}

/// `include_builtin_actions = true` on a project policy is honoured by the
/// public `evaluate` entry point, not only by `preset`.
const PROJECT_POLICY: &str = r#"
name = "project"
description = "Project fixture"
class = "project"
schema = 1
include_builtin_actions = true

[[bands]]
id = "none"
label = "None"
rank = 0
dispositions = ["close", "accept"]

[[bands]]
id = "act"
label = "Act"
rank = 1
dispositions = ["mitigate", "accept"]

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["none", "none", "act"],
  ["none", "act",  "act"],
  ["act",  "act",  "act"],
]

[signoff]
max_residual_band = "act"
include_kernel_gate = true
"#;

#[test]
fn evaluate_merges_builtin_catalog_for_a_project_policy() {
    let policy = parse_policy(PROJECT_POLICY).unwrap();
    validate(&policy).unwrap();
    let loaded = LoadedPolicy {
        hash: policy_hash(&policy),
        policy,
        source: PolicySource::Project,
    };
    let s = evaluate(&golden_open(), &loaded, as_of()).unwrap();
    let ids: BTreeSet<&str> = s
        .failure_modes
        .iter()
        .flat_map(|fm| fm.actions.iter().map(|a| a.id.as_str()))
        .collect();
    let catalog_actions = builtin_actions();
    let catalog: BTreeSet<&str> = catalog_actions.iter().map(|a| a.id.as_str()).collect();
    assert!(
        ids.intersection(&catalog).next().is_some(),
        "a project policy with include_builtin_actions must receive catalog actions"
    );
}

// --- confidence-view reads rows from the real `standard` preset ------------

/// A confidence policy whose own S×P matrix maps every cell to its lowest band
/// (`a`), so the effective band can only come from the real `standard` preset's
/// effective band rank (row 1 for a mainstream `(2,2)` residual).
const STANDARD_ROW_POLICY: &str = r#"
name = "standard-row"
description = "Confidence fixture"
class = "standard"
schema = 1

[[bands]]
id = "a"
label = "A"
rank = 0
dispositions = ["close", "accept"]

[[bands]]
id = "b"
label = "B"
rank = 1
dispositions = ["mitigate"]

[[bands]]
id = "c"
label = "C"
rank = 2
dispositions = ["mitigate"]

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["a", "a", "a"],
  ["a", "a", "a"],
  ["a", "a", "a"],
]

[matrices.confidence]
derived_from = "standard-bands"
cells = [
  ["a", "a", "a", "a", "a"],
  ["b", "b", "b", "b", "b"],
  ["c", "c", "c", "c", "c"],
]

[signoff]
max_residual_band = "c"
"#;

#[test]
fn confidence_policy_reads_its_row_from_the_standard_preset() {
    let policy = parse_policy(STANDARD_ROW_POLICY).unwrap();
    validate(&policy).unwrap();
    let loaded = LoadedPolicy {
        hash: policy_hash(&policy),
        policy,
        source: PolicySource::Project,
    };
    let subject = subject(
        MatrixStrategy::Qualitative3x3,
        None,
        vec![open_fm("fm1", Domain::Runtime, false, (2, 2))],
    );
    let s = evaluate(&subject, &loaded, as_of()).unwrap();
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("b"));
}

// --- compat-0.1 parity with the kernel readiness gate ------------------------

fn afm(id: &str, severity: &str, probability: &str) -> AnalyzeInput {
    AnalyzeInput {
        id: id.into(),
        component: EntityRef::new("c1"),
        description: "d".into(),
        cause: Some("c".into()),
        effect: Some("e".into()),
        domain: Domain::Runtime,
        scope: None,
        severity_observations: vec![severity.into()],
        probability_observations: vec![probability.into()],
        mitigations: Vec::new(),
    }
}

fn analyze_fixture(strategy: MatrixStrategy, input: Vec<AnalyzeInput>) -> (Subject, bool) {
    let report = fmeca::analyze(strategy, &input).unwrap();
    let subject = subject_from_analysis(strategy, &input).unwrap();
    (subject, report.ready)
}

fn session_fixture(log: &str) -> (Subject, bool) {
    let events: Vec<fmeca::Event> = log
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let state = fmeca::projection::project(&events);
    let ready = state.readiness.ready;
    (subject_from_state(&state), ready)
}

const ACCEPTED_LOG: &str = r#"
{"type":"session_opened","session_id":"s1","matrix_strategy":"qualitative3x3"}
{"type":"failure_mode_added","failure_mode":{"id":"fm-acc","session_id":"s1","component":{"id":"c1"},"description":"d","cause":"c","effect":"e","severity_observations":["data_loss"],"probability_observations":["happens_in_normal_use"],"domain":"runtime","source":{"turn_id":"t1"}}}
{"type":"failure_mode_dispositioned","change":{"session_id":"s1","failure_mode_id":"fm-acc","disposition":"accepted","rationale":"known and bounded","approver":"lead"}}
"#;

const EXCLUDED_LOG: &str = r#"
{"type":"session_opened","session_id":"s1","matrix_strategy":"qualitative3x3"}
{"type":"failure_mode_added","failure_mode":{"id":"fm-bad","session_id":"s1","component":{"id":"c1"},"description":"d","cause":"c","effect":"e","severity_observations":["data_loss"],"probability_observations":["happens_in_normal_use"],"domain":"runtime","source":{"turn_id":"t1"}}}
{"type":"failure_mode_dispositioned","change":{"session_id":"s1","failure_mode_id":"fm-bad","disposition":"excluded","rationale":"not credible","evidence":[{"kind":"test","reference":"t"}]}}
{"type":"failure_mode_added","failure_mode":{"id":"fm-good","session_id":"s1","component":{"id":"c1"},"description":"d","cause":"c","effect":"e","severity_observations":["cosmetic"],"probability_observations":["rare_edge_case"],"domain":"runtime","source":{"turn_id":"t1"}}}
"#;

fn compat_fixtures() -> Vec<(&'static str, Subject, bool)> {
    let mut fixtures = Vec::new();

    let (s, r) = analyze_fixture(
        MatrixStrategy::Qualitative3x3,
        vec![afm("fm-low", "cosmetic", "rare_edge_case")],
    );
    fixtures.push(("analyze-3x3-low", s, r));

    let (s, r) = analyze_fixture(
        MatrixStrategy::Qualitative3x3,
        vec![afm("fm-high", "data_loss", "happens_in_normal_use")],
    );
    fixtures.push(("analyze-3x3-high", s, r));

    let (s, r) = analyze_fixture(
        MatrixStrategy::Nasa8004_5x5,
        vec![afm("fm-low", "negligible_impact", "improbable")],
    );
    fixtures.push(("analyze-5x5-low", s, r));

    let (s, r) = analyze_fixture(
        MatrixStrategy::Nasa8004_5x5,
        vec![afm("fm-high", "loss_of_life_or_mission", "near_certain")],
    );
    fixtures.push(("analyze-5x5-high", s, r));

    let (s, r) = analyze_fixture(
        MatrixStrategy::Qualitative3x3,
        vec![AnalyzeInput {
            severity_observations: Vec::new(),
            probability_observations: Vec::new(),
            ..afm("fm-unscored", "cosmetic", "rare_edge_case")
        }],
    );
    fixtures.push(("analyze-unscored", s, r));

    let mut mitigated = afm("fm-mitigated", "data_loss", "happens_in_normal_use");
    mitigated.mitigations.push(AnalyzeMitigation {
        id: "m1".into(),
        kind: MitigationKind::Prevention,
        description: "fix".into(),
        residual_severity_observations: vec!["cosmetic".into()],
        residual_probability_observations: vec!["rare_edge_case".into()],
    });
    let (s, r) = analyze_fixture(MatrixStrategy::Qualitative3x3, vec![mitigated]);
    fixtures.push(("analyze-mitigated", s, r));

    let (s, r) = session_fixture(ACCEPTED_LOG);
    fixtures.push(("session-accepted", s, r));

    let (s, r) = session_fixture(EXCLUDED_LOG);
    fixtures.push(("session-excluded", s, r));

    fixtures
}

#[test]
fn compat_preset_matches_readiness_on_all_fixtures() {
    let compat = preset("compat-0.1").expect("compat preset");
    let mismatches: Vec<String> = compat_fixtures()
        .into_iter()
        .filter_map(|(name, subject, ready)| {
            let pass = evaluate(&subject, &compat, as_of()).unwrap().signoff.pass;
            (pass != ready).then(|| format!("{name}: compat pass={pass} but kernel ready={ready}"))
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "compat-0.1 must equal the kernel gate:\n{}",
        mismatches.join("\n")
    );
}

// --- golden outputs ----------------------------------------------------------

fn open_fm(id: &str, domain: Domain, sp: bool, inherent: (u8, u8)) -> SubjectFm {
    SubjectFm {
        id: id.into(),
        domain,
        single_point: sp,
        inherent: Some(inherent),
        residual: None,
        response_class: None,
        disposition: Disposition::Open,
        disposition_rationale: None,
        accepted: None,
        mitigations: Vec::new(),
    }
}

fn evidence(kind: fmeca::EvidenceKind, basis: Option<fmeca::EvidenceBasis>) -> ResolutionEvidence {
    ResolutionEvidence {
        kind,
        reference: "ref".into(),
        basis,
    }
}

fn mitigation(
    id: &str,
    kind: MitigationKind,
    status: MitigationStatus,
    residual: (u8, u8),
    evidence: Vec<ResolutionEvidence>,
) -> SubjectMitigation {
    SubjectMitigation {
        id: id.into(),
        kind,
        status,
        residual: Some(residual),
        evidence,
    }
}

fn subject(strategy: MatrixStrategy, level: Option<AnalysisLevel>, fms: Vec<SubjectFm>) -> Subject {
    Subject {
        strategy,
        level,
        failure_modes: fms,
        kernel_gate: KernelGate {
            ready: true,
            blockers: Vec::new(),
        },
    }
}

fn golden_open() -> Subject {
    let mut mitigated = open_fm("fm-b", Domain::Security, false, (2, 2));
    mitigated.residual = Some((1, 1));
    mitigated.mitigations.push(mitigation(
        "m1",
        MitigationKind::Prevention,
        MitigationStatus::Implemented,
        (1, 1),
        vec![evidence(fmeca::EvidenceKind::Commit, None)],
    ));
    subject(
        MatrixStrategy::Qualitative3x3,
        Some(AnalysisLevel::Implementation),
        vec![open_fm("fm-a", Domain::Runtime, true, (3, 3)), mitigated],
    )
}

fn golden_mitigated() -> Subject {
    let mut fm = open_fm("fm-m", Domain::Delivery, false, (3, 2));
    fm.residual = Some((2, 2));
    fm.response_class = Some(fmeca::ResponseClass::Restructure);
    fm.mitigations.push(mitigation(
        "m1",
        MitigationKind::Detection,
        MitigationStatus::Verified,
        (2, 2),
        vec![evidence(
            fmeca::EvidenceKind::Test,
            Some(fmeca::EvidenceBasis::Test),
        )],
    ));
    subject(MatrixStrategy::Qualitative3x3, None, vec![fm])
}

fn golden_mixed() -> Subject {
    let mut accepted = open_fm("fm-acc", Domain::Runtime, false, (5, 5));
    accepted.disposition = Disposition::Accepted;
    accepted.accepted = Some(Accepted {
        approver: "lead".into(),
        at: Some("2026-01-01T00:00:00Z".into()),
        rationale: "known".into(),
    });
    let mut excluded = open_fm("fm-exc", Domain::Architecture, false, (4, 4));
    excluded.disposition = Disposition::Excluded;
    excluded.disposition_rationale = Some("not credible".into());
    let unscored = SubjectFm {
        id: "fm-unscored".into(),
        domain: Domain::Ux,
        single_point: false,
        inherent: None,
        residual: None,
        response_class: None,
        disposition: Disposition::Open,
        disposition_rationale: None,
        accepted: None,
        mitigations: Vec::new(),
    };
    subject(
        MatrixStrategy::Nasa8004_5x5,
        None,
        vec![accepted, excluded, unscored],
    )
}

fn golden_fixtures() -> [(&'static str, Subject); 3] {
    [
        ("open", golden_open()),
        ("mitigated", golden_mitigated()),
        ("mixed", golden_mixed()),
    ]
}

fn golden_mismatch(preset_name: &str, fixture: &str, subject: &Subject) -> Option<String> {
    let loaded = preset(preset_name).expect("preset");
    let suggestions = evaluate(subject, &loaded, as_of()).unwrap();
    let actual = format!("{}\n", serde_json::to_string_pretty(&suggestions).unwrap());
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{preset_name}-{fixture}.json"));
    if std::env::var_os("FMECA_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return None;
    }
    match std::fs::read_to_string(&path) {
        Ok(expected) if expected == actual => None,
        Ok(_) => Some(format!(
            "{} differs; regenerate with FMECA_BLESS=1",
            path.display()
        )),
        Err(_) => Some(format!(
            "{} missing; run with FMECA_BLESS=1",
            path.display()
        )),
    }
}

fn golden_for(preset_name: &str) -> Vec<String> {
    golden_fixtures()
        .iter()
        .filter_map(|(fixture, subject)| golden_mismatch(preset_name, fixture, subject))
        .collect()
}

#[test]
fn golden_compat_preset_matches_three_fixtures() {
    assert!(golden_for("compat-0.1").is_empty());
}

#[test]
fn golden_prototype_preset_matches_three_fixtures() {
    assert!(golden_for("prototype").is_empty());
}

#[test]
fn golden_standard_preset_matches_three_fixtures() {
    assert!(golden_for("standard").is_empty());
}

#[test]
fn golden_business_critical_preset_matches_three_fixtures() {
    assert!(golden_for("business-critical").is_empty());
}

#[test]
fn golden_safety_critical_preset_matches_three_fixtures() {
    assert!(golden_for("safety-critical").is_empty());
}

#[test]
fn golden_confidence_view_preset_matches_three_fixtures() {
    assert!(golden_for("confidence-view").is_empty());
}

// --- docs --------------------------------------------------------------------

#[test]
fn docs_list_every_preset() {
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/policies.md"
    ))
    .expect("docs/policies.md");
    let missing: Vec<&str> = PRESET_NAMES
        .iter()
        .copied()
        .filter(|name| !doc.contains(name))
        .collect();
    assert!(missing.is_empty(), "docs/policies.md missing: {missing:?}");
}
