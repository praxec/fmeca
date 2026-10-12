//! Three-level effects (local / next / end) and single-point critical items.

mod common;

use common::{disposition_change, evidence, failure_mode, temp_engine};
use fmeca::Disposition::Excluded;
use fmeca::EvidenceKind::Test;
use fmeca::Level::{High, Low};
use fmeca::{Engine, Event, FailureMode};

fn single_point_session(
    severity: fmeca::Level,
    probability: fmeca::Level,
) -> (Engine, tempfile::TempDir) {
    let (engine, dir) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = failure_mode("s", "fm-1", severity, probability);
    fm.single_point = true;
    engine.add_failure_mode("s", fm).unwrap();
    (engine, dir)
}

#[test]
fn old_failure_mode_without_new_fields_replays() {
    let line = r#"{"type":"failure_mode_added","failure_mode":{"id":"fm-1","session_id":"s","component":{"id":"comp:svc"},"description":"d","severity_observations":[],"probability_observations":[],"domain":"runtime","source":{"turn_id":"t1"}}}"#;
    let event: Event = serde_json::from_str(line).unwrap();
    let fm: FailureMode = match event {
        Event::FailureModeAdded { failure_mode } => *failure_mode,
        _ => unreachable!("line is a failure_mode_added event"),
    };
    assert_eq!(
        (
            fm.local_effect,
            fm.next_effect,
            fm.single_point,
            fm.parent,
            fm.refs
        ),
        (None, None, false, None, Vec::<String>::new())
    );
}

#[test]
fn local_and_next_effects_round_trip() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = failure_mode("s", "fm-1", High, High);
    fm.local_effect = Some("request fails".to_string());
    fm.next_effect = Some("checkout retries".to_string());
    let state = engine.add_failure_mode("s", fm).unwrap();
    let got = &state.failure_modes[0].failure_mode;
    assert_eq!(
        (got.local_effect.as_deref(), got.next_effect.as_deref()),
        (Some("request fails"), Some("checkout retries"))
    );
}

#[test]
fn single_point_high_is_a_critical_item() {
    let (engine, _d) = single_point_session(High, High);
    assert_eq!(engine.readiness("s").unwrap().critical_items, vec!["fm-1"]);
}

#[test]
fn single_point_low_is_not_a_critical_item() {
    let (engine, _d) = single_point_session(Low, Low);
    assert!(engine.readiness("s").unwrap().critical_items.is_empty());
}

#[test]
fn excluded_single_point_is_not_a_critical_item() {
    let (engine, _d) = single_point_session(High, High);
    engine
        .set_failure_mode_disposition(
            "s",
            disposition_change("s", "fm-1", Excluded, None, vec![evidence(Test, "t::x")]),
        )
        .unwrap();
    assert!(engine.readiness("s").unwrap().critical_items.is_empty());
}

#[test]
fn export_lists_critical_items() {
    let (engine, _d) = single_point_session(High, High);
    assert_eq!(engine.export("s").unwrap().critical_items, vec!["fm-1"]);
}
