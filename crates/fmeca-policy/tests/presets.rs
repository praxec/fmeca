//! Built-in presets, the shared action catalog, golden outputs and
//! `docs/policies.md` (spec §6, plan Task 3).
//!
//! Every test makes one behavioural assertion. Goldens are compared
//! byte-for-byte; regenerate them only with `FMECA_BLESS=1`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use fmeca::{
    AnalysisLevel, AnalyzeInput, AnalyzeMitigation, Disposition, DispositionChange, Domain, Engine,
    EntityRef, EvidenceRef, FailureMode, FmecaState, MatrixStrategy, Mitigation, MitigationKind,
    MitigationStatus, ResolutionEvidence, StateStore,
};
use fmeca_policy::presets::{PRESET_NAMES, builtin_actions, preset, resolve};
use fmeca_policy::{
    Accepted, Conditions, Confidence, Date, EvidenceMode, KernelGate, LoadedPolicy, PolicyError,
    PolicySource, STANDARD_BAND_COUNT, Subject, SubjectFm, SubjectMitigation, Suggestions,
    evaluate, parse_date, parse_policy, policy_hash, subject_from_analysis, subject_from_state,
    validate,
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
    assert!(matches!(
        resolve("no-such-policy", None),
        Err(PolicyError::NotFound(_))
    ));
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

/// The 0.1 kernel bucket each compat band stands for: `Low` → `none`,
/// `Medium`/`High` → `mitigate`.
fn compat_band_for(criticality: fmeca::Criticality) -> &'static str {
    match criticality {
        fmeca::Level::Low => "none",
        fmeca::Level::Medium | fmeca::Level::High => "mitigate",
    }
}

#[test]
fn compat_cells_equal_kernel_criticality() {
    let compat = preset("compat-0.1").expect("compat preset");
    let mut mismatches = Vec::new();
    for strategy in [MatrixStrategy::Qualitative3x3, MatrixStrategy::Nasa8004_5x5] {
        let cells = &compat.policy.matrices[strategy.id()].cells;
        let n = strategy.level_count();
        for s in 1..=n {
            for p in 1..=n {
                let expected = compat_band_for(strategy.criticality(s, p));
                let actual = cells[usize::from(s - 1)][usize::from(p - 1)].as_str();
                if actual != expected {
                    mismatches.push(format!(
                        "{} ({s},{p}): {actual} but kernel says {expected}",
                        strategy.id()
                    ));
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

/// An in-memory event store, so the session path runs through the real
/// [`fmeca::Engine`] without touching the filesystem.
#[derive(Default)]
struct MemStore(Mutex<BTreeMap<String, Vec<fmeca::Event>>>);

impl StateStore for MemStore {
    fn append(&self, session_id: &str, event: &fmeca::Event) -> fmeca::Result<()> {
        self.0
            .lock()
            .unwrap()
            .entry(session_id.to_string())
            .or_default()
            .push(event.clone());
        Ok(())
    }

    fn replay(&self, session_id: &str) -> fmeca::Result<Vec<fmeca::Event>> {
        self.0
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .ok_or_else(|| fmeca::FmecaError::SessionNotFound(session_id.to_string()))
    }

    fn exists(&self, session_id: &str) -> fmeca::Result<bool> {
        Ok(self.0.lock().unwrap().contains_key(session_id))
    }
}

/// One parity fixture: failure modes as `analyze` input, plus dispositions
/// recorded on the session path only (`AnalyzeInput` carries none).
struct ParityFixture {
    name: &'static str,
    strategy: MatrixStrategy,
    input: Vec<AnalyzeInput>,
    /// `(failure-mode id, Accepted | Excluded)`.
    dispositions: Vec<(&'static str, Disposition)>,
    /// `(mitigation id, status, evidence)`, recorded on the session path only.
    resolutions: Vec<(&'static str, MitigationStatus, Vec<ResolutionEvidence>)>,
}

impl ParityFixture {
    fn new(name: &'static str, strategy: MatrixStrategy, input: Vec<AnalyzeInput>) -> Self {
        Self {
            name,
            strategy,
            input,
            dispositions: Vec::new(),
            resolutions: Vec::new(),
        }
    }

    fn resolved(
        mut self,
        mitigation_id: &'static str,
        status: MitigationStatus,
        evidence: Vec<ResolutionEvidence>,
    ) -> Self {
        self.resolutions.push((mitigation_id, status, evidence));
        self
    }

    fn with(mut self, id: &'static str, disposition: Disposition) -> Self {
        self.dispositions.push((id, disposition));
        self
    }

    /// The same failure modes built as a session through the engine.
    fn session(&self) -> FmecaState {
        const SID: &str = "parity";
        let engine = Engine::new(Arc::new(MemStore::default()));
        engine.open_session_with(SID, self.strategy).unwrap();
        for input in &self.input {
            engine
                .add_failure_mode(
                    SID,
                    FailureMode {
                        id: input.id.clone(),
                        session_id: SID.into(),
                        component: input.component.clone(),
                        description: input.description.clone(),
                        cause: input.cause.clone(),
                        effect: input.effect.clone(),
                        severity_observations: input.severity_observations.clone(),
                        probability_observations: input.probability_observations.clone(),
                        domain: input.domain,
                        scope: input.scope,
                        source: EvidenceRef::new("turn-1"),
                        parent: None,
                        refs: Vec::new(),
                        local_effect: None,
                        next_effect: None,
                        single_point: false,
                    },
                )
                .unwrap();
            for m in &input.mitigations {
                engine
                    .add_mitigation(
                        SID,
                        Mitigation {
                            id: m.id.clone(),
                            session_id: SID.into(),
                            failure_mode_id: input.id.clone(),
                            kind: m.kind,
                            description: m.description.clone(),
                            residual_severity_observations: m
                                .residual_severity_observations
                                .clone(),
                            residual_probability_observations: m
                                .residual_probability_observations
                                .clone(),
                            source: EvidenceRef::new("turn-2"),
                        },
                    )
                    .unwrap();
            }
        }
        for (id, status, evidence) in &self.resolutions {
            engine
                .set_mitigation_status(
                    SID,
                    fmeca::MitigationStatusChange {
                        session_id: SID.into(),
                        mitigation_id: (*id).into(),
                        status: *status,
                        evidence: evidence.clone(),
                        note: None,
                        at: Some("2026-10-01T00:00:00Z".into()),
                    },
                )
                .unwrap();
        }
        for (id, disposition) in &self.dispositions {
            engine
                .set_failure_mode_disposition(
                    SID,
                    DispositionChange {
                        session_id: SID.into(),
                        failure_mode_id: (*id).into(),
                        disposition: *disposition,
                        rationale: "known and bounded".into(),
                        approver: Some("lead".into()),
                        evidence: vec![evidence(fmeca::EvidenceKind::Test, None)],
                        at: Some("2026-10-01T00:00:00Z".into()),
                    },
                )
                .unwrap();
        }
        engine.state(SID).unwrap()
    }
}

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

fn mitigated_afm(
    id: &str,
    (severity, probability): (&str, &str),
    kind: MitigationKind,
    (residual_severity, residual_probability): (&str, &str),
) -> AnalyzeInput {
    let mut input = afm(id, severity, probability);
    input.mitigations.push(AnalyzeMitigation {
        id: format!("{id}-m1"),
        kind,
        description: "fix".into(),
        residual_severity_observations: vec![residual_severity.into()],
        residual_probability_observations: vec![residual_probability.into()],
    });
    input
}

/// Low, Medium and High cells, mitigated and kernel-blocked cases on both
/// strategies, plus unscored, empty, accepted and excluded analyses.
fn parity_fixtures() -> Vec<ParityFixture> {
    use MatrixStrategy::{Nasa8004_5x5 as N5, Qualitative3x3 as Q3};
    const HIGH_3: (&str, &str) = ("data_loss", "happens_in_normal_use");
    const LOW_3: (&str, &str) = ("cosmetic", "rare_edge_case");
    const HIGH_5: (&str, &str) = ("loss_of_life_or_mission", "near_certain");
    const MEDIUM_5: (&str, &str) = ("degraded_capability", "occasional");
    const LOW_5: (&str, &str) = ("negligible_impact", "improbable");
    let unscored = AnalyzeInput {
        severity_observations: Vec::new(),
        probability_observations: Vec::new(),
        ..afm("fm-unscored", "cosmetic", "rare_edge_case")
    };
    vec![
        ParityFixture::new("3x3-low", Q3, vec![afm("fm-low", LOW_3.0, LOW_3.1)]),
        ParityFixture::new(
            "3x3-medium",
            Q3,
            vec![afm("fm-medium", "user_facing_degradation", "occasional")],
        ),
        ParityFixture::new("3x3-high", Q3, vec![afm("fm-high", HIGH_3.0, HIGH_3.1)]),
        ParityFixture::new(
            "3x3-mitigated",
            Q3,
            vec![mitigated_afm(
                "fm-m",
                HIGH_3,
                MitigationKind::Prevention,
                LOW_3,
            )],
        ),
        ParityFixture::new(
            "3x3-mitigated-to-medium",
            Q3,
            vec![mitigated_afm(
                "fm-m",
                HIGH_3,
                MitigationKind::Detection,
                ("recoverable_error", "occasional"),
            )],
        ),
        // Low residual, but fail_fast only: the kernel's DISCIPLINE blocker.
        ParityFixture::new(
            "3x3-fail-fast-only",
            Q3,
            vec![mitigated_afm(
                "fm-ff",
                HIGH_3,
                MitigationKind::FailFast,
                LOW_3,
            )],
        ),
        ParityFixture::new("3x3-unscored", Q3, vec![unscored.clone()]),
        // Accepted but unscored: 0.1 readiness raises no CLARIFY (not open).
        ParityFixture::new("3x3-accepted-unscored", Q3, vec![unscored])
            .with("fm-unscored", Disposition::Accepted),
        ParityFixture::new("3x3-empty", Q3, Vec::new()),
        ParityFixture::new("5x5-low", N5, vec![afm("fm-low", LOW_5.0, LOW_5.1)]),
        ParityFixture::new(
            "5x5-medium",
            N5,
            vec![afm("fm-medium", MEDIUM_5.0, MEDIUM_5.1)],
        ),
        ParityFixture::new("5x5-high", N5, vec![afm("fm-high", HIGH_5.0, HIGH_5.1)]),
        ParityFixture::new(
            "5x5-mitigated",
            N5,
            vec![mitigated_afm(
                "fm-m",
                HIGH_5,
                MitigationKind::Prevention,
                LOW_5,
            )],
        ),
        ParityFixture::new(
            "5x5-mitigated-to-medium",
            N5,
            vec![mitigated_afm(
                "fm-m",
                HIGH_5,
                MitigationKind::Prevention,
                MEDIUM_5,
            )],
        ),
        ParityFixture::new("3x3-accepted", Q3, vec![afm("fm-acc", HIGH_3.0, HIGH_3.1)])
            .with("fm-acc", Disposition::Accepted),
        ParityFixture::new(
            "3x3-excluded",
            Q3,
            vec![
                afm("fm-bad", HIGH_3.0, HIGH_3.1),
                afm("fm-good", LOW_3.0, LOW_3.1),
            ],
        )
        .with("fm-bad", Disposition::Excluded),
        ParityFixture::new(
            "5x5-accepted-and-low",
            N5,
            vec![
                afm("fm-acc", HIGH_5.0, HIGH_5.1),
                afm("fm-low", LOW_5.0, LOW_5.1),
            ],
        )
        .with("fm-acc", Disposition::Accepted),
    ]
}

#[test]
fn parity_fixtures_cover_every_bucket_and_a_mitigation_on_both_strategies() {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for fixture in parity_fixtures() {
        let state = fixture.session();
        for view in &state.failure_modes {
            let strategy = fixture.strategy.id();
            if let Some(c) = view.residual_criticality {
                seen.insert(format!("{strategy} {c:?}"));
            }
            if !view.mitigations.is_empty() {
                seen.insert(format!("{strategy} mitigated"));
            }
        }
    }
    let missing: Vec<String> = ["qualitative3x3", "nasa8004_5x5"]
        .iter()
        .flat_map(|s| ["Low", "Medium", "High", "mitigated"].map(|c| format!("{s} {c}")))
        .filter(|key| !seen.contains(key))
        .collect();
    assert!(missing.is_empty(), "uncovered: {missing:?}");
}

/// Review Focus 5. Per fixture, every verdict available must agree: the
/// one-shot `fmeca::analyze(..).ready`, the session-path `readiness.ready`,
/// and `compat-0.1`'s `signoff.pass` on both the session and the analysis
/// subject. Fixtures with recorded dispositions have no `analyze` form.
#[test]
fn compat_preset_matches_readiness_on_all_fixtures() {
    let compat = preset("compat-0.1").expect("compat preset");
    let mut mismatches = Vec::new();
    for fixture in parity_fixtures() {
        let state = fixture.session();
        let mut verdicts = vec![
            ("session readiness.ready", state.readiness.ready),
            (
                "compat pass (session)",
                evaluate(&subject_from_state(&state), &compat, as_of())
                    .unwrap()
                    .signoff
                    .pass,
            ),
        ];
        if fixture.dispositions.is_empty() {
            let report = fmeca::analyze(fixture.strategy, &fixture.input).unwrap();
            let subject = subject_from_analysis(fixture.strategy, &fixture.input).unwrap();
            verdicts.push(("analyze.ready", report.ready));
            verdicts.push((
                "compat pass (analysis)",
                evaluate(&subject, &compat, as_of()).unwrap().signoff.pass,
            ));
        }
        if verdicts.iter().any(|(_, v)| *v != verdicts[0].1) {
            mismatches.push(format!("{}: {verdicts:?}", fixture.name));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

/// I1: an accepted, unscored failure mode is ready under 0.1 readiness, and
/// `compat-0.1` (`unscored_fails = false`) passes it too.
#[test]
fn compat_passes_accepted_unscored_session_that_readiness_passes() {
    let fixture = parity_fixtures()
        .into_iter()
        .find(|f| f.name == "3x3-accepted-unscored")
        .expect("fixture");
    let state = fixture.session();
    let compat = preset("compat-0.1").expect("compat preset");
    let pass = evaluate(&subject_from_state(&state), &compat, as_of())
        .unwrap()
        .signoff
        .pass;
    assert_eq!((state.readiness.ready, pass), (true, true));
}

#[test]
fn only_compat_turns_off_unscored_fails() {
    let off: Vec<&str> = PRESET_NAMES
        .iter()
        .copied()
        .filter(|name| preset(name).is_some_and(|lp| !lp.policy.signoff.unscored_fails))
        .collect();
    assert_eq!(off, vec!["compat-0.1"]);
}

/// With the kernel gate switched off, compat-0.1's band rule alone must equal
/// the kernel's residual rule: no open (not accepted, not excluded) failure
/// mode with a Medium or High residual. Fixtures with an unscored failure mode
/// are skipped: the evaluator fails those under its separate `unscored` rule.
#[test]
fn compat_band_verdict_matches_kernel_band_rule() {
    let mut band_only = preset("compat-0.1").expect("compat preset");
    band_only.policy.signoff.include_kernel_gate = false;
    band_only.hash = policy_hash(&band_only.policy);
    let mut mismatches = Vec::new();
    for fixture in parity_fixtures() {
        let state = fixture.session();
        let considered: Vec<&fmeca::FailureModeView> = state
            .failure_modes
            .iter()
            .filter(|v| v.disposition != Disposition::Excluded)
            .collect();
        if considered.iter().any(|v| v.criticality.is_none()) {
            continue;
        }
        let kernel_rule = !considered.iter().any(|v| {
            v.disposition == Disposition::Open
                && matches!(
                    v.residual_criticality,
                    Some(fmeca::Level::Medium | fmeca::Level::High)
                )
        });
        let pass = evaluate(&subject_from_state(&state), &band_only, as_of())
            .unwrap()
            .signoff
            .pass;
        if pass != kernel_rule {
            mismatches.push(format!(
                "{}: band-only pass={pass}, kernel band rule={kernel_rule}",
                fixture.name
            ));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

// --- the kernel gate carries hygiene, not the residual zone (C1) -------------

/// One open, unmitigated, fully described failure mode at 3×3 severity
/// `data_loss` × probability `rare_edge_case`: kernel Medium (a RESIDUAL
/// blocker), `standard` `compensate`, `prototype` `report`.
fn medium_analysis() -> Vec<AnalyzeInput> {
    vec![afm("fm-mid", "data_loss", "rare_edge_case")]
}

/// The sign-off of `medium_analysis` under `preset_name`, through the real
/// `fmeca::analyze` kernel gate.
fn medium_signoff(preset_name: &str) -> fmeca_policy::SignoffVerdict {
    let subject =
        subject_from_analysis(MatrixStrategy::Qualitative3x3, &medium_analysis()).unwrap();
    let loaded = preset(preset_name).expect("preset");
    evaluate(&subject, &loaded, as_of()).unwrap().signoff
}

#[test]
fn standard_passes_unmitigated_compensate_band_with_real_kernel_gate() {
    let signoff = medium_signoff("standard");
    assert!(signoff.pass, "{:?}", signoff.reasons);
}

#[test]
fn prototype_passes_unmitigated_report_band_with_real_kernel_gate() {
    let signoff = medium_signoff("prototype");
    assert!(signoff.pass, "{:?}", signoff.reasons);
}

#[test]
fn kernel_gate_leaves_out_residual_blockers() {
    let subject =
        subject_from_analysis(MatrixStrategy::Qualitative3x3, &medium_analysis()).unwrap();
    assert_eq!(
        subject.kernel_gate,
        KernelGate {
            ready: true,
            blockers: Vec::new()
        }
    );
}

#[test]
fn kernel_gate_keeps_hygiene_blockers() {
    // Low residual, but fail_fast only: the kernel's DISCIPLINE blocker stays.
    let input = vec![mitigated_afm(
        "fm-ff",
        ("data_loss", "happens_in_normal_use"),
        MitigationKind::FailFast,
        ("cosmetic", "rare_edge_case"),
    )];
    let subject = subject_from_analysis(MatrixStrategy::Qualitative3x3, &input).unwrap();
    assert_eq!(
        (
            subject.kernel_gate.ready,
            subject.kernel_gate.blockers.len()
        ),
        (false, 1)
    );
}

// --- evidence modes: the fallback threshold (C2) ------------------------------

/// The first 5×5 cell (row order) of `band` in `loaded`.
fn cell_of(loaded: &LoadedPolicy, band: &str) -> (u8, u8) {
    let cells = &loaded.policy.matrices["nasa8004_5x5"].cells;
    cells
        .iter()
        .enumerate()
        .find_map(|(s, row)| {
            row.iter()
                .position(|b| b == band)
                .map(|p| (s as u8 + 1, p as u8 + 1))
        })
        .expect("band has a cell")
}

/// Evidence reaching exactly `confidence`.
fn evidence_at(confidence: Confidence) -> Vec<ResolutionEvidence> {
    use fmeca::{EvidenceBasis as B, EvidenceKind as K};
    match confidence {
        Confidence::C0 => Vec::new(),
        Confidence::C1 => vec![evidence(K::File, None)],
        Confidence::C2 => vec![evidence(K::Commit, None)],
        Confidence::C3 => vec![evidence(K::Test, None)],
        Confidence::C4 => vec![evidence(K::Url, Some(B::OperationalUse))],
    }
}

/// `preset_name` with its `evidence_mode` set to `mode` (and re-hashed).
fn preset_in(preset_name: &str, mode: EvidenceMode) -> LoadedPolicy {
    let mut loaded = preset(preset_name).expect("preset");
    loaded.policy.evidence_mode = mode;
    loaded.hash = policy_hash(&loaded.policy);
    loaded
}

/// One failure mode with inherent `from` and an implemented prevention
/// claiming `to`, its evidence at `confidence`, evaluated under `preset_name`
/// (default evidence mode) on the 5×5 strategy.
fn claimed(preset_name: &str, from: &str, to: &str, confidence: Confidence) -> Suggestions {
    claimed_in(&preset(preset_name).expect("preset"), from, to, confidence)
}

/// [`claimed`] under an already-loaded policy (e.g. one from [`preset_in`]).
fn claimed_in(loaded: &LoadedPolicy, from: &str, to: &str, confidence: Confidence) -> Suggestions {
    claimed_cells(
        loaded,
        cell_of(loaded, from),
        cell_of(loaded, to),
        confidence,
    )
}

/// [`claimed_in`] with the inherent and residual 5×5 cells given directly.
fn claimed_cells(
    loaded: &LoadedPolicy,
    inherent: (u8, u8),
    residual: (u8, u8),
    confidence: Confidence,
) -> Suggestions {
    let mut fm = open_fm("fm1", Domain::Runtime, false, inherent);
    fm.residual = Some(residual);
    fm.mitigations.push(mitigation(
        "m1",
        MitigationKind::Prevention,
        MitigationStatus::Implemented,
        residual,
        evidence_at(confidence),
    ));
    let subject = subject(MatrixStrategy::Nasa8004_5x5, None, vec![fm]);
    evaluate(&subject, loaded, as_of()).unwrap()
}

#[test]
fn claimed_reduction_to_lowest_band_needs_the_left_bands_evidence_under_standard() {
    // Strong mode: leaving `review` needs C2; C1 evidence falls back.
    let strong = preset_in("standard", EvidenceMode::Strong);
    let s = claimed_in(&strong, "review", "none", Confidence::C1);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("review"));
}

#[test]
fn claimed_reduction_to_lowest_band_needs_the_left_bands_evidence_under_safety_critical() {
    // Strong mode: leaving `high` needs C3; C2 evidence falls back.
    let strong = preset_in("safety-critical", EvidenceMode::Strong);
    let s = claimed_in(&strong, "high", "low", Confidence::C2);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("high"));
}

#[test]
fn standard_mode_reduction_to_lowest_band_needs_the_first_gated_band_left() {
    // `review` → `none` at C0: below `compensate`'s C1, so it falls back.
    let s = claimed("standard", "review", "none", Confidence::C0);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("review"));
}

#[test]
fn standard_mode_reduction_with_first_gated_band_evidence_passes() {
    // `review` → `none` at C1 meets `compensate`'s C1: no fallback.
    let s = claimed("standard", "review", "none", Confidence::C1);
    assert!(s.failure_modes[0].fallback.is_none());
}

#[test]
fn strong_mode_requires_every_left_band_so_c1_falls_back_under_strong() {
    let strong = preset_in("standard", EvidenceMode::Strong);
    let s = claimed_in(&strong, "review", "none", Confidence::C1);
    assert!(s.failure_modes[0].fallback.is_some());
}

#[test]
fn strong_mode_requires_every_left_band_so_c1_counts_under_standard() {
    let s = claimed("standard", "review", "none", Confidence::C1);
    assert!(s.failure_modes[0].fallback.is_none());
}

#[test]
fn confidence_view_standard_row_fallback_follows_its_own_evidence_mode() {
    // `standard`'s `review` → `none` at C1 counts under the default mode;
    // a strong-mode `confidence-view` makes the standard row fall back.
    let standard = preset("standard").expect("standard preset");
    let strong_view = preset_in("confidence-view", EvidenceMode::Strong);
    let s = claimed_cells(
        &strong_view,
        cell_of(&standard, "review"),
        cell_of(&standard, "none"),
        Confidence::C1,
    );
    assert!(s.failure_modes[0].fallback.is_some());
}

#[test]
fn safety_critical_unevidenced_full_reduction_falls_back_in_standard_mode() {
    // `high` → `low` at C0: `low` sets no `min_confidence`, but `medium`
    // (C2), the first gated band left, does.
    let s = claimed("safety-critical", "high", "low", Confidence::C0);
    assert_eq!(s.failure_modes[0].effective_band.as_deref(), Some("high"));
}

#[test]
fn full_reduction_with_sufficient_evidence_passes() {
    let s = claimed("safety-critical", "high", "low", Confidence::C3);
    assert!(s.signoff.pass, "{:?}", s.signoff.reasons);
}

/// The lowest confidence at which a claimed `from` → `to` reduction counts
/// (no fallback) under `loaded`; 5 when even C4 falls back.
fn needed_confidence(loaded: &LoadedPolicy, from: &str, to: &str) -> usize {
    const LADDER: [Confidence; 5] = [
        Confidence::C0,
        Confidence::C1,
        Confidence::C2,
        Confidence::C3,
        Confidence::C4,
    ];
    LADDER
        .iter()
        .position(|&c| {
            claimed_in(loaded, from, to, c).failure_modes[0]
                .fallback
                .is_none()
        })
        .unwrap_or(LADDER.len())
}

/// Under strong mode, for every preset with its own S×P bands
/// (`confidence-view` takes its fallback from `standard`), every inherent
/// band and every pair of lower residual bands: the bigger claimed reduction
/// never needs less evidence.
#[test]
fn fallback_threshold_is_monotone_in_claimed_reduction() {
    let mut violations = Vec::new();
    for name in PRESET_NAMES.iter().filter(|n| **n != "confidence-view") {
        let strong = preset_in(name, EvidenceMode::Strong);
        let mut bands = strong.policy.bands.clone();
        bands.sort_by_key(|b| b.rank);
        for (i, inherent) in bands.iter().enumerate() {
            for (j, bigger) in bands[..i].iter().enumerate() {
                for smaller in &bands[j + 1..i] {
                    let big = needed_confidence(&strong, &inherent.id, &bigger.id);
                    let small = needed_confidence(&strong, &inherent.id, &smaller.id);
                    if big < small {
                        violations.push(format!(
                            "{name}: {} → {} needs C{big} < {} → {} needs C{small}",
                            inherent.id, bigger.id, inherent.id, smaller.id
                        ));
                    }
                }
            }
        }
    }
    assert!(violations.is_empty(), "{violations:#?}");
}

// --- safety-critical against MIL-STD-882E Table III --------------------------

/// MIL-STD-882E Table III: rows probability A (Frequent) .. E (Improbable),
/// columns severity I (Catastrophic) .. IV (Negligible).
const TABLE_III: [[&str; 4]; 5] = [
    ["high", "high", "serious", "medium"],
    ["high", "high", "serious", "medium"],
    ["high", "serious", "medium", "low"],
    ["serious", "medium", "medium", "low"],
    ["medium", "medium", "medium", "low"],
];

#[test]
fn safety_critical_matches_mil_std_882e_table_iii() {
    let safety = preset("safety-critical").expect("safety-critical preset");
    let cells = &safety.policy.matrices["nasa8004_5x5"].cells;
    let mut mismatches = Vec::new();
    for (i, row) in TABLE_III.iter().enumerate() {
        for (j, expected) in row.iter().enumerate() {
            // Probability A..E is column ordinal 5..1; severity I..IV is row
            // ordinal 5..2.
            let (s, p) = (5 - j, 5 - i);
            let actual = &cells[s - 1][p - 1];
            if actual != expected {
                let letter = ["A", "B", "C", "D", "E"][i];
                let roman = ["I", "II", "III", "IV"][j];
                mismatches.push(format!(
                    "{letter}/{roman} (row {s}, column {p}): {actual}, Table III says {expected}"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

/// The 3×3 analogue is the 5×5's middle: 3×3 severity and probability
/// ordinal `k` is 5×5 ordinal `k + 1` (IV/III/II and D/C/B).
#[test]
fn safety_critical_3x3_is_the_middle_of_the_5x5() {
    let safety = preset("safety-critical").expect("safety-critical preset");
    let small = &safety.policy.matrices["qualitative3x3"].cells;
    let large = &safety.policy.matrices["nasa8004_5x5"].cells;
    let mismatches: Vec<String> = (0..3)
        .flat_map(|s| (0..3).map(move |p| (s, p)))
        .filter(|&(s, p)| small[s][p] != large[s + 1][p + 1])
        .map(|(s, p)| format!("({}, {})", s + 1, p + 1))
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:?}");
}

// --- confidence-view: the maintainer's chart 2 -------------------------------

#[test]
fn confidence_view_cells_match_chart_2() {
    let view = preset("confidence-view").expect("confidence-view preset");
    let expected = [
        ["none", "none", "none", "report", "report"],
        ["none", "report", "report", "remediate", "remediate"],
        ["report", "remediate", "remediate", "remediate", "remediate"],
    ];
    assert_eq!(view.policy.matrices["confidence"].cells, expected);
}

// --- required-action tailoring -----------------------------------------------

/// One open, unmitigated, single-point runtime failure mode in the 3×3
/// High/High cell, analysed at the implementation level.
fn worst_case() -> Subject {
    subject(
        MatrixStrategy::Qualitative3x3,
        Some(AnalysisLevel::Implementation),
        vec![open_fm("fm1", Domain::Runtime, true, (3, 3))],
    )
}

fn required_ids(preset_name: &str, subject: &Subject) -> Vec<String> {
    let loaded = preset(preset_name).expect("preset");
    evaluate(subject, &loaded, as_of()).unwrap().failure_modes[0]
        .actions
        .iter()
        .filter(|a| a.required)
        .map(|a| a.id.clone())
        .collect()
}

#[test]
fn every_tailored_required_action_becomes_required() {
    let tailoring = [
        ("standard", "design-review"),
        ("business-critical", "design-review"),
        ("business-critical", "prevention-before-detection"),
        ("business-critical", "mitigate-single-point"),
        ("safety-critical", "design-review"),
        ("safety-critical", "prevention-before-detection"),
        ("safety-critical", "test-and-inspection-evidence"),
    ];
    let subject = worst_case();
    let missing: Vec<String> = tailoring
        .iter()
        .filter(|(p, id)| !required_ids(p, &subject).iter().any(|r| r == id))
        .map(|(p, id)| format!("{p}: {id}"))
        .collect();
    assert!(missing.is_empty(), "not required: {missing:?}");
}

#[test]
fn business_critical_accepted_single_point_in_review_needs_mitigation() {
    let mut fm = open_fm("fm1", Domain::Runtime, true, (3, 3));
    fm.disposition = Disposition::Accepted;
    fm.accepted = Some(Accepted {
        approver: "lead".into(),
        at: Some("2026-10-01T00:00:00Z".into()),
        rationale: "known".into(),
    });
    let loaded = preset("business-critical").expect("preset");
    let s = evaluate(
        &subject(MatrixStrategy::Qualitative3x3, None, vec![fm]),
        &loaded,
        as_of(),
    )
    .unwrap();
    assert!(
        s.rollups
            .required_uncovered
            .contains(&("fm1".to_string(), "mitigate-single-point".to_string()))
    );
}

#[test]
fn business_critical_single_point_covered_by_implemented_prevention() {
    let mut fm = open_fm("fm1", Domain::Runtime, true, (3, 3));
    fm.mitigations.push(mitigation(
        "m1",
        MitigationKind::Prevention,
        MitigationStatus::Implemented,
        (3, 3),
        vec![evidence(fmeca::EvidenceKind::Test, None)],
    ));
    fm.residual = Some((3, 3));
    let loaded = preset("business-critical").expect("preset");
    let s = evaluate(
        &subject(MatrixStrategy::Qualitative3x3, None, vec![fm]),
        &loaded,
        as_of(),
    )
    .unwrap();
    assert!(
        !s.rollups
            .required_uncovered
            .iter()
            .any(|(_, action)| action == "mitigate-single-point")
    );
}

#[test]
fn planned_prevention_does_not_lift_prevention_before_detection() {
    // business-critical `review` (3×3 worst cell) with only a planned
    // prevention: a plan is not coverage, so prevention is still missing.
    let mut fm = open_fm("fm1", Domain::Runtime, false, (3, 3));
    fm.residual = Some((1, 1));
    fm.mitigations.push(mitigation(
        "m1",
        MitigationKind::Prevention,
        MitigationStatus::Planned,
        (1, 1),
        Vec::new(),
    ));
    let s = evaluate(
        &subject(MatrixStrategy::Qualitative3x3, None, vec![fm]),
        &preset("business-critical").expect("preset"),
        as_of(),
    )
    .unwrap();
    assert!(
        s.rollups
            .required_uncovered
            .contains(&("fm1".to_string(), "prevention-before-detection".to_string()))
    );
}

#[test]
fn flag_procedural_only_fires_only_with_fail_fast_only_mitigation() {
    let with = |id: &str, kind: MitigationKind| {
        let mut fm = open_fm(id, Domain::Runtime, false, (3, 3));
        fm.mitigations.push(mitigation(
            &format!("{id}-m"),
            kind,
            MitigationStatus::Implemented,
            (3, 3),
            Vec::new(),
        ));
        fm.residual = Some((3, 3));
        fm
    };
    let subject = subject(
        MatrixStrategy::Qualitative3x3,
        None,
        vec![
            with("fm-ff", MitigationKind::FailFast),
            with("fm-prev", MitigationKind::Prevention),
            open_fm("fm-none", Domain::Runtime, false, (3, 3)),
        ],
    );
    let loaded = preset("safety-critical").expect("preset");
    let flagged: Vec<String> = evaluate(&subject, &loaded, as_of())
        .unwrap()
        .failure_modes
        .into_iter()
        .filter(|f| f.actions.iter().any(|a| a.id == "flag-procedural-only"))
        .map(|f| f.id)
        .collect();
    assert_eq!(flagged, vec!["fm-ff".to_string()]);
}

#[test]
fn safety_critical_accepted_high_risk_fails_signoff() {
    let s = evaluate(
        &golden_mixed(),
        &preset("safety-critical").expect("preset"),
        as_of(),
    )
    .unwrap();
    assert!(
        s.signoff
            .reasons
            .contains(&"fm-acc: accepted in band 'high' which does not allow acceptance".into())
    );
}

/// Only low-cost hygiene actions reach a failure mode in a preset's lowest
/// band.
#[test]
fn lowest_band_failure_modes_get_only_hygiene_actions() {
    const HYGIENE: [&str; 2] = ["secure-defaults", "adr-for-accepted-risk"];
    let mut low = open_fm("fm-low", Domain::Runtime, false, (1, 1));
    low.mitigations.push(mitigation(
        "m1",
        MitigationKind::Prevention,
        MitigationStatus::Verified,
        (1, 1),
        vec![evidence(
            fmeca::EvidenceKind::Test,
            Some(fmeca::EvidenceBasis::OperationalUse),
        )],
    ));
    low.residual = Some((1, 1));
    let mut fms = golden_open().failure_modes;
    fms.push(low);
    let subject = subject(
        MatrixStrategy::Qualitative3x3,
        Some(AnalysisLevel::Implementation),
        fms,
    );
    let mut extra = Vec::new();
    for name in PRESET_NAMES {
        let loaded = preset(name).expect("preset");
        let lowest = loaded
            .policy
            .bands
            .iter()
            .min_by_key(|b| b.rank)
            .map(|b| b.id.clone());
        for fm in evaluate(&subject, &loaded, as_of()).unwrap().failure_modes {
            if fm.effective_band != lowest {
                continue;
            }
            for action in fm
                .actions
                .iter()
                .filter(|a| !HYGIENE.contains(&a.id.as_str()))
            {
                extra.push(format!("{name} {}: {}", fm.id, action.id));
            }
        }
    }
    assert!(extra.is_empty(), "{extra:#?}");
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
    // Worst cell claimed down to the best by an implemented prevention with
    // commit evidence (C2). In the default (standard) evidence mode that
    // meets the first gated band left: `standard`'s `compensate` (C1),
    // `business-critical`'s `compensate` (C2) and `safety-critical`'s
    // `medium` (C2), so no preset falls back.
    let mut claimed = open_fm("fm-claimed", Domain::Runtime, false, (5, 5));
    claimed.residual = Some((1, 1));
    claimed.mitigations.push(mitigation(
        "m1",
        MitigationKind::Prevention,
        MitigationStatus::Implemented,
        (1, 1),
        vec![evidence(fmeca::EvidenceKind::Commit, None)],
    ));
    let mut mixed = subject(
        MatrixStrategy::Nasa8004_5x5,
        None,
        vec![accepted, claimed, excluded, unscored],
    );
    // The kernel's own verdict on an open unscored failure mode.
    mixed.kernel_gate = KernelGate {
        ready: false,
        blockers: vec!["CLARIFY: failure mode 'fm-unscored' is missing cause/effect/score".into()],
    };
    mixed
}

/// A real session through the engine, so its kernel gate is the kernel's own
/// (not a stub): `fm-mid` is an unmitigated kernel-Medium failure mode (a
/// RESIDUAL blocker the gate leaves out), and `fm-fixed` a High one prevented
/// down to Low by an implemented mitigation with test evidence (C3).
fn golden_session() -> Subject {
    let fixture = ParityFixture::new(
        "golden-session",
        MatrixStrategy::Qualitative3x3,
        vec![
            afm("fm-mid", "data_loss", "rare_edge_case"),
            mitigated_afm(
                "fm-fixed",
                ("data_loss", "happens_in_normal_use"),
                MitigationKind::Prevention,
                ("cosmetic", "rare_edge_case"),
            ),
        ],
    )
    .resolved(
        "fm-fixed-m1",
        MitigationStatus::Implemented,
        vec![evidence(fmeca::EvidenceKind::Test, None)],
    );
    subject_from_state(&fixture.session())
}

/// Every golden fixture. `session` carries a real kernel gate; `mixed` stubs
/// the kernel's CLARIFY blocker for its open unscored failure mode; the others
/// stub a ready, empty one.
fn golden_fixtures() -> [(&'static str, Subject); 4] {
    [
        ("open", golden_open()),
        ("mitigated", golden_mitigated()),
        ("mixed", golden_mixed()),
        ("session", golden_session()),
    ]
}

fn golden_mismatch(preset_name: &str, fixture: &str, subject: &Subject) -> Option<String> {
    let loaded = preset(preset_name).expect("preset");
    let suggestions = evaluate(subject, &loaded, as_of()).unwrap();
    let actual = format!("{}\n", serde_json::to_string_pretty(&suggestions).unwrap());
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{preset_name}-{fixture}.json"));
    if std::env::var("FMECA_BLESS").as_deref() == Ok("1") {
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
fn golden_compat_preset_matches_its_golden_fixtures() {
    let mismatches = golden_for("compat-0.1");
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn golden_prototype_preset_matches_its_golden_fixtures() {
    let mismatches = golden_for("prototype");
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn golden_standard_preset_matches_its_golden_fixtures() {
    let mismatches = golden_for("standard");
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn golden_business_critical_preset_matches_its_golden_fixtures() {
    let mismatches = golden_for("business-critical");
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn golden_safety_critical_preset_matches_its_golden_fixtures() {
    let mismatches = golden_for("safety-critical");
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn golden_confidence_view_preset_matches_its_golden_fixtures() {
    let mismatches = golden_for("confidence-view");
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

// --- docs --------------------------------------------------------------------

/// `docs/policies.md` has a section per preset and states that a `note` is
/// C0 whatever its basis.
#[test]
fn docs_list_every_preset() {
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/policies.md"
    ))
    .expect("docs/policies.md");
    let headings: BTreeSet<&str> = doc
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .map(str::trim)
        .collect();
    let mut missing: Vec<String> = PRESET_NAMES
        .iter()
        .filter(|name| !headings.contains(*name))
        .map(|name| format!("### {name}"))
        .collect();
    const NOTE_SENTENCE: &str = "A **`note` is C0 whatever its basis**";
    if !doc.contains(NOTE_SENTENCE) {
        missing.push(NOTE_SENTENCE.to_string());
    }
    assert!(missing.is_empty(), "docs/policies.md missing: {missing:?}");
}
