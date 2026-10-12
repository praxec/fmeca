//! Markdown rendering: rank ordering, excluded failure modes, required actions.

use fmeca::{Disposition, Domain, MatrixStrategy};
use fmeca_policy::{
    KernelGate, LoadedPolicy, PolicySource, Subject, SubjectFm, evaluate, parse_date, parse_policy,
    policy_hash, preset, render, validate,
};

/// A project policy whose band ids sort opposite to their ranks: `a-low`
/// (rank 0) sorts before `z-high` (rank 1) alphabetically.
const PROJECT_POLICY: &str = r#"
name = "team"
description = "Two bands"
class = "standard"
schema = 1

[[bands]]
id = "a-low"
label = "Low"
rank = 0
dispositions = ["close"]

[[bands]]
id = "z-high"
label = "High"
rank = 1
dispositions = ["mitigate"]

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["a-low", "a-low", "z-high"],
  ["a-low", "z-high", "z-high"],
  ["z-high", "z-high", "z-high"],
]

[signoff]
max_residual_band = "a-low"
"#;

fn policy_from(text: &str) -> LoadedPolicy {
    let policy = parse_policy(text).unwrap();
    validate(&policy).unwrap();
    LoadedPolicy {
        hash: policy_hash(&policy),
        policy,
        source: PolicySource::Project,
    }
}

fn fm(id: &str, inherent: (u8, u8)) -> SubjectFm {
    SubjectFm {
        id: id.into(),
        domain: Domain::Runtime,
        single_point: false,
        inherent: Some(inherent),
        residual: None,
        response_class: None,
        disposition: Disposition::Open,
        disposition_rationale: None,
        accepted: None,
        mitigations: vec![],
    }
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

fn as_of() -> fmeca_policy::Date {
    parse_date("2026-10-12").unwrap()
}

#[test]
fn markdown_lists_required_actions_first() {
    let loaded = preset("business-critical").unwrap();
    let suggestions = evaluate(&subject(vec![fm("fm-review", (3, 3))]), &loaded, as_of()).unwrap();
    let markdown = render::to_markdown(&suggestions, &loaded.policy);
    assert!(
        markdown.find("prevention-before-detection").unwrap()
            < markdown.find("simplify-design").unwrap(),
        "required `prevention-before-detection` must be listed before suggested `simplify-design`:\n{markdown}"
    );
}

#[test]
fn markdown_orders_bands_by_rank_for_project_policy() {
    let loaded = policy_from(PROJECT_POLICY);
    let suggestions = evaluate(
        &subject(vec![fm("fm-a", (1, 1)), fm("fm-z", (3, 3))]),
        &loaded,
        as_of(),
    )
    .unwrap();
    let markdown = render::to_markdown(&suggestions, &loaded.policy);
    assert!(
        markdown.find("## z-high").unwrap() < markdown.find("## a-low").unwrap(),
        "rank 1 band `z-high` must precede rank 0 band `a-low`:\n{markdown}"
    );
}

#[test]
fn markdown_lists_excluded_failure_modes() {
    let loaded = preset("compat-0.1").unwrap();
    let mut excluded = fm("fm-excluded", (3, 3));
    excluded.disposition = Disposition::Excluded;
    excluded.disposition_rationale = Some("not credible".into());
    let suggestions = evaluate(&subject(vec![excluded]), &loaded, as_of()).unwrap();
    let markdown = render::to_markdown(&suggestions, &loaded.policy);
    assert!(
        markdown.find("- `fm-excluded`: not credible").unwrap()
            < markdown.find("## Sign-off").unwrap(),
        "excluded entries must be listed with their rationale before sign-off:\n{markdown}"
    );
}
