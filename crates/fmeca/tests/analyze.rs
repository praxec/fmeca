//! Table + parity tests for the stateless one-shot [`fmeca::analyze`].
//!
//! The headline guarantee is *compute-parity*: `analyze` must produce the SAME
//! criticality / residual / standing / readiness as the session path for an
//! equivalent input. `analyze_agrees_with_session_path` LOCKS that by building
//! the identical FMECA through the `Engine` and asserting equality.

mod common;

use common::{failure_mode_obs, temp_engine};
use fmeca::{
    analyze, AnalyzeInput, AnalyzeMitigation, Criticality, Domain, EntityRef, FailureModeStanding,
    Level, MatrixStrategy, Mitigation, MitigationKind, ResponseClass, Scope,
};

/// A scored 3×3 failure mode built from observation ids that derive to the given
/// raw S/P levels.
fn input_3x3(
    id: &str,
    severity: Level,
    probability: Level,
    domain: Domain,
    mitigations: Vec<AnalyzeMitigation>,
) -> AnalyzeInput {
    AnalyzeInput {
        id: id.to_string(),
        component: EntityRef::new(format!("comp:{id}")),
        description: format!("failure mode {id}"),
        cause: Some("a cause".to_string()),
        effect: Some("an effect".to_string()),
        domain,
        scope: None,
        severity_observations: vec![sev_obs(severity)],
        probability_observations: vec![prob_obs(probability)],
        mitigations,
    }
}

fn sev_obs(level: Level) -> String {
    match level {
        Level::High => "data_loss",
        Level::Medium => "user_facing_degradation",
        Level::Low => "cosmetic",
    }
    .to_string()
}

fn prob_obs(level: Level) -> String {
    match level {
        Level::High => "happens_in_normal_use",
        Level::Medium => "occasional",
        Level::Low => "rare_edge_case",
    }
    .to_string()
}

fn prevention_to_low(id: &str) -> AnalyzeMitigation {
    AnalyzeMitigation {
        id: id.to_string(),
        kind: MitigationKind::Prevention,
        description: "drive residual to low".to_string(),
        // Residual is observation-derived: cosmetic→low severity,
        // rare_edge_case→low probability ⇒ Low residual under the 3×3 matrix.
        residual_severity_observations: vec![sev_obs(Level::Low)],
        residual_probability_observations: vec![prob_obs(Level::Low)],
    }
}

#[test]
fn empty_batch_is_trivially_not_ready_but_reports_cleanly() {
    let report = analyze(MatrixStrategy::Qualitative3x3, &[]).unwrap();
    assert!(report.failure_modes.is_empty());
    assert!(report.risk_ranking.is_empty());
    assert!(report.issues.is_empty());
    // An FMECA with nothing analyzed is not "ready" — same rule as the gate.
    assert!(!report.ready);
    assert!(report.blockers.iter().any(|b| b.starts_with("EMPTY:")));
}

#[test]
fn mixed_batch_computes_per_fm_criticality_and_ranking() {
    let batch = vec![
        // Low/Low → Low.
        input_3x3("low", Level::Low, Level::Low, Domain::Runtime, vec![]),
        // High/High → High, unmitigated.
        input_3x3("high", Level::High, Level::High, Domain::Runtime, vec![]),
        // Medium/Medium → Medium, unmitigated.
        input_3x3("med", Level::Medium, Level::Medium, Domain::Ux, vec![]),
    ];
    let report = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();

    let by_id = |id: &str| {
        report
            .failure_modes
            .iter()
            .find(|f| f.id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id("low").criticality, Some(Criticality::Low));
    assert_eq!(by_id("high").criticality, Some(Criticality::High));
    assert_eq!(by_id("med").criticality, Some(Criticality::Medium));

    // standing: low acceptable, high+med unmitigated.
    assert_eq!(by_id("low").standing, Some(FailureModeStanding::Acceptable));
    assert_eq!(
        by_id("high").standing,
        Some(FailureModeStanding::Unmitigated)
    );

    // ranking: high, med, low (criticality desc).
    assert_eq!(report.risk_ranking, vec!["high", "med", "low"]);

    // not ready: residual High/Medium stand.
    assert!(!report.ready);
    assert!(report.blockers.iter().any(|b| b.starts_with("RESIDUAL:")));
}

#[test]
fn response_class_is_computed_per_fm() {
    let batch = vec![
        input_3x3(
            "arch",
            Level::High,
            Level::High,
            Domain::Architecture,
            vec![],
        ),
        input_3x3("rt", Level::High, Level::High, Domain::Runtime, vec![]),
    ];
    let report = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();
    let arch = report
        .failure_modes
        .iter()
        .find(|f| f.id == "arch")
        .unwrap();
    let rt = report.failure_modes.iter().find(|f| f.id == "rt").unwrap();
    assert_eq!(arch.response_class, Some(ResponseClass::ReArchitecture));
    assert_eq!(rt.response_class, Some(ResponseClass::Restructure));
}

#[test]
fn mitigation_dropping_residual_to_low_flips_ready() {
    // A single High mode, fully mitigated to Low → ready.
    let batch = vec![input_3x3(
        "fm1",
        Level::High,
        Level::High,
        Domain::Runtime,
        vec![prevention_to_low("m1")],
    )];
    let report = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();
    let fm = &report.failure_modes[0];
    assert_eq!(fm.criticality, Some(Criticality::High));
    assert_eq!(fm.residual_criticality, Some(Criticality::Low));
    assert_eq!(fm.standing, Some(FailureModeStanding::Acceptable));
    assert!(report.ready);
    assert!(report.blockers.is_empty());
}

#[test]
fn nasa_5x5_strategy_collapses_to_lmh_and_ranks() {
    let mk = |id: &str, sev: &str, prob: &str| AnalyzeInput {
        id: id.to_string(),
        component: EntityRef::new(format!("comp:{id}")),
        description: format!("nasa fm {id}"),
        cause: Some("c".to_string()),
        effect: Some("e".to_string()),
        domain: Domain::Runtime,
        scope: None,
        severity_observations: vec![sev.to_string()],
        probability_observations: vec![prob.to_string()],
        mitigations: vec![],
    };
    let batch = vec![
        // catastrophic + near-certain (5,5) → High.
        mk("worst", "loss_of_life_or_mission", "near_certain"),
        // negligible + improbable (1,1) → Low.
        mk("least", "negligible_impact", "improbable"),
    ];
    let report = analyze(MatrixStrategy::Nasa8004_5x5, &batch).unwrap();
    assert_eq!(report.matrix_strategy, MatrixStrategy::Nasa8004_5x5);
    let worst = report
        .failure_modes
        .iter()
        .find(|f| f.id == "worst")
        .unwrap();
    let least = report
        .failure_modes
        .iter()
        .find(|f| f.id == "least")
        .unwrap();
    assert_eq!(worst.criticality, Some(Criticality::High));
    assert_eq!(least.criticality, Some(Criticality::Low));
    assert_eq!(report.risk_ranking, vec!["worst", "least"]);
}

#[test]
fn unknown_observation_id_is_invalid_observation() {
    let batch = vec![AnalyzeInput {
        id: "fm1".to_string(),
        component: EntityRef::new("comp:svc"),
        description: "x".to_string(),
        cause: Some("c".to_string()),
        effect: Some("e".to_string()),
        domain: Domain::Runtime,
        scope: None,
        severity_observations: vec!["not_a_real_id".to_string()],
        probability_observations: vec!["happens_in_normal_use".to_string()],
        mitigations: vec![],
    }];
    let err = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap_err();
    assert!(matches!(err, fmeca::FmecaError::InvalidObservation(_)));
    assert!(err.to_string().starts_with("INVALID_OBSERVATION:"));
}

#[test]
fn cross_strategy_observation_is_rejected() {
    // A NASA id under the 3×3 strategy is unknown.
    let batch = vec![AnalyzeInput {
        id: "fm1".to_string(),
        component: EntityRef::new("comp:svc"),
        description: "x".to_string(),
        cause: Some("c".to_string()),
        effect: Some("e".to_string()),
        domain: Domain::Runtime,
        scope: None,
        severity_observations: vec!["loss_of_life_or_mission".to_string()],
        probability_observations: vec![],
        mitigations: vec![],
    }];
    let err = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap_err();
    assert!(matches!(err, fmeca::FmecaError::InvalidObservation(_)));
}

#[test]
fn weak_mitigation_order_blocks_readiness() {
    // High mode mitigated to Low but ONLY via fail_fast → weak_mitigation_order
    // blocks readiness even though residual is Low.
    let batch = vec![input_3x3(
        "fm1",
        Level::High,
        Level::High,
        Domain::Runtime,
        vec![AnalyzeMitigation {
            id: "m1".to_string(),
            kind: MitigationKind::FailFast,
            description: "crash loudly".to_string(),
            residual_severity_observations: vec![sev_obs(Level::Low)],
            residual_probability_observations: vec![prob_obs(Level::Low)],
        }],
    )];
    let report = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();
    let fm = &report.failure_modes[0];
    assert_eq!(fm.residual_criticality, Some(Criticality::Low));
    assert!(!report.ready);
    assert!(report.blockers.iter().any(|b| b.starts_with("DISCIPLINE:")));
}

#[test]
fn missing_cause_and_score_produce_clarify_blockers() {
    let batch = vec![AnalyzeInput {
        id: "fm1".to_string(),
        component: EntityRef::new("comp:svc"),
        description: "incomplete".to_string(),
        cause: None,
        effect: Some("e".to_string()),
        domain: Domain::Runtime,
        scope: None,
        severity_observations: vec![], // unscored severity
        probability_observations: vec!["occasional".to_string()],
        mitigations: vec![],
    }];
    let report = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();
    assert!(!report.ready);
    assert!(report.blockers.iter().any(|b| b.starts_with("CLARIFY:")));
    let fm = &report.failure_modes[0];
    assert_eq!(fm.criticality, None);
    // per-FM issues carry the missing_cause + missing_score gaps.
    assert!(fm
        .issues
        .iter()
        .any(|i| i.r#type == fmeca::IssueType::MissingCause));
    assert!(fm
        .issues
        .iter()
        .any(|i| i.r#type == fmeca::IssueType::MissingScore));
}

#[test]
fn analyze_is_idempotent() {
    let batch = vec![
        input_3x3("a", Level::High, Level::Medium, Domain::Runtime, vec![]),
        input_3x3(
            "b",
            Level::High,
            Level::High,
            Domain::Runtime,
            vec![prevention_to_low("mb")],
        ),
    ];
    let first = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();
    let second = analyze(MatrixStrategy::Qualitative3x3, &batch).unwrap();
    assert_eq!(first, second);
}

/// COMPUTE-PARITY (the no-divergence lock): build the SAME FMECA via the `Engine`
/// session path and assert `analyze` produces identical criticality / residual /
/// standing / response_class per failure mode and identical readiness. If the
/// two paths ever computed differently, this fails.
#[test]
fn analyze_agrees_with_session_path() {
    let strategy = MatrixStrategy::Nasa8004_5x5;

    // --- the stateless analyze input -----------------------------------------
    let fm_worst = AnalyzeInput {
        id: "fm-worst".to_string(),
        component: EntityRef::new("comp:svc"),
        description: "failure mode fm-worst".to_string(),
        cause: Some("a cause".to_string()),
        effect: Some("an effect".to_string()),
        domain: Domain::Runtime,
        scope: None,
        severity_observations: vec!["loss_of_life_or_mission".to_string()],
        probability_observations: vec!["near_certain".to_string()],
        mitigations: vec![AnalyzeMitigation {
            id: "mit-1".to_string(),
            kind: MitigationKind::Prevention,
            description: "mitigation mit-1".to_string(),
            // NASA residual observations: degraded_capability (consequence 3) +
            // occasional (likelihood 3) ⇒ (3,3) collapses to Medium.
            residual_severity_observations: vec!["degraded_capability".to_string()],
            residual_probability_observations: vec!["occasional".to_string()],
        }],
    };
    let fm_mid = AnalyzeInput {
        id: "fm-mid".to_string(),
        component: EntityRef::new("comp:svc"),
        description: "failure mode fm-mid".to_string(),
        cause: Some("a cause".to_string()),
        effect: Some("an effect".to_string()),
        domain: Domain::Architecture,
        scope: Some(Scope::Structural),
        severity_observations: vec!["degraded_capability".to_string()],
        probability_observations: vec!["occasional".to_string()],
        mitigations: vec![],
    };
    let batch = vec![fm_worst, fm_mid];
    let report = analyze(strategy, &batch).unwrap();

    // --- the SAME FMECA via the Engine session path --------------------------
    let (engine, _dir) = temp_engine();
    engine.open_session_with("parity", strategy).unwrap();

    // fm-worst: built from the same observations; same mitigation.
    let mut fm0 = failure_mode_obs(
        "parity",
        "fm-worst",
        vec!["loss_of_life_or_mission".to_string()],
        vec!["near_certain".to_string()],
    );
    fm0.component = EntityRef::new("comp:svc");
    fm0.domain = Domain::Runtime;
    engine.add_failure_mode("parity", fm0).unwrap();
    engine
        .add_mitigation(
            "parity",
            Mitigation {
                id: "mit-1".to_string(),
                session_id: "parity".to_string(),
                failure_mode_id: "fm-worst".to_string(),
                kind: MitigationKind::Prevention,
                description: "mitigation mit-1".to_string(),
                residual_severity_observations: vec!["degraded_capability".to_string()],
                residual_probability_observations: vec!["occasional".to_string()],
                source: fmeca::EvidenceRef::new("t2"),
            },
        )
        .unwrap();

    // fm-mid.
    let mut fm1 = failure_mode_obs(
        "parity",
        "fm-mid",
        vec!["degraded_capability".to_string()],
        vec!["occasional".to_string()],
    );
    fm1.component = EntityRef::new("comp:svc");
    fm1.domain = Domain::Architecture;
    fm1.scope = Some(Scope::Structural);
    let session_state = engine.add_failure_mode("parity", fm1).unwrap();

    // --- assert parity per failure mode --------------------------------------
    for view in &session_state.failure_modes {
        let computed = report
            .failure_modes
            .iter()
            .find(|f| f.id == view.failure_mode.id)
            .unwrap_or_else(|| panic!("analyze missing {}", view.failure_mode.id));
        assert_eq!(
            computed.criticality, view.criticality,
            "criticality parity for {}",
            view.failure_mode.id
        );
        assert_eq!(
            computed.residual_criticality, view.residual_criticality,
            "residual parity for {}",
            view.failure_mode.id
        );
        assert_eq!(
            computed.standing, view.standing,
            "standing parity for {}",
            view.failure_mode.id
        );
        assert_eq!(
            computed.response_class, view.response_class,
            "response_class parity for {}",
            view.failure_mode.id
        );
    }

    // readiness parity.
    assert_eq!(report.ready, session_state.readiness.ready);
    assert_eq!(report.blockers, session_state.readiness.blockers);
    // issue parity (same detectors over the same projection).
    assert_eq!(report.issues, session_state.issues);
}
