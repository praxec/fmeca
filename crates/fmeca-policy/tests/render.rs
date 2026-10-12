//! Markdown rendering: required actions are listed before suggested ones.

use fmeca::{Disposition, Domain, MatrixStrategy};
use fmeca_policy::{KernelGate, Subject, SubjectFm, evaluate, parse_date, preset, render};

fn subject() -> Subject {
    Subject {
        strategy: MatrixStrategy::Qualitative3x3,
        level: None,
        failure_modes: vec![SubjectFm {
            id: "fm-review".into(),
            domain: Domain::Runtime,
            single_point: false,
            inherent: Some((3, 3)),
            residual: None,
            response_class: None,
            disposition: Disposition::Open,
            disposition_rationale: None,
            accepted: None,
            mitigations: vec![],
        }],
        kernel_gate: KernelGate {
            ready: true,
            blockers: vec![],
        },
    }
}

#[test]
fn markdown_lists_required_actions_first() {
    let loaded = preset("business-critical").unwrap();
    let suggestions = evaluate(&subject(), &loaded, parse_date("2026-10-12").unwrap()).unwrap();
    let markdown = render::to_markdown(&suggestions);
    assert!(
        markdown.find("prevention-before-detection").unwrap()
            < markdown.find("simplify-design").unwrap(),
        "required `prevention-before-detection` must be listed before suggested `simplify-design`:\n{markdown}"
    );
}
