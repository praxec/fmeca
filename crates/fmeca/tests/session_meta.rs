//! Analysis metadata (`SessionMeta`), session and failure-mode links, and
//! session listing.

mod common;

use common::{failure_mode, temp_engine};
use fmeca::Level::High;
use fmeca::{AnalysisLevel, Event, MatrixStrategy, ParentRef, SessionMeta, projection};

fn meta(level: AnalysisLevel, aspect: &str) -> SessionMeta {
    SessionMeta {
        level: Some(level),
        aspect: Some(aspect.to_string()),
        ..SessionMeta::default()
    }
}

#[test]
fn old_log_without_meta_replays_with_default_meta() {
    let line = r#"{"type":"session_opened","session_id":"old","matrix_strategy":"qualitative3x3"}"#;
    let event: Event = serde_json::from_str(line).unwrap();
    assert_eq!(projection::project(&[event]).meta, SessionMeta::default());
}

#[test]
fn session_meta_is_recorded_and_returned() {
    let (engine, _d) = temp_engine();
    let state = engine
        .open_session_with_meta(
            "s",
            MatrixStrategy::default(),
            meta(AnalysisLevel::TechnicalPlan, "checkout"),
        )
        .unwrap();
    assert_eq!(state.meta.level, Some(AnalysisLevel::TechnicalPlan));
}

#[test]
fn reopening_does_not_change_meta() {
    let (engine, _d) = temp_engine();
    engine
        .open_session_with_meta(
            "s",
            MatrixStrategy::default(),
            meta(AnalysisLevel::Concept, "checkout"),
        )
        .unwrap();
    let state = engine
        .open_session_with_meta(
            "s",
            MatrixStrategy::default(),
            meta(AnalysisLevel::Verification, "billing"),
        )
        .unwrap();
    assert_eq!(state.meta, meta(AnalysisLevel::Concept, "checkout"));
}

#[test]
fn parent_session_must_exist() {
    let (engine, _d) = temp_engine();
    let child = SessionMeta {
        parent: Some("nope".to_string()),
        ..SessionMeta::default()
    };
    let err = engine
        .open_session_with_meta("child", MatrixStrategy::default(), child)
        .unwrap_err();
    assert!(matches!(err, fmeca::FmecaError::SessionNotFound(ref p) if p == "nope"));
}

#[test]
fn failure_mode_parent_must_exist() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = failure_mode("s", "fm-1", High, High);
    fm.parent = Some(ParentRef {
        session_id: "s".to_string(),
        failure_mode_id: "fm-x".to_string(),
    });
    let err = engine.add_failure_mode("s", fm).unwrap_err();
    assert!(
        matches!(err, fmeca::FmecaError::InvalidFailureMode(ref m) if m.contains("parent failure mode not found"))
    );
}

#[test]
fn failure_mode_parent_in_other_session_is_accepted() {
    let (engine, _d) = temp_engine();
    engine.open_session("p").unwrap();
    engine
        .add_failure_mode("p", failure_mode("p", "fm-p", High, High))
        .unwrap();
    engine.open_session("c").unwrap();
    let mut fm = failure_mode("c", "fm-c", High, High);
    fm.parent = Some(ParentRef {
        session_id: "p".to_string(),
        failure_mode_id: "fm-p".to_string(),
    });
    assert!(engine.add_failure_mode("c", fm).is_ok());
}

#[test]
fn existing_parent_session_is_accepted() {
    let (engine, _d) = temp_engine();
    engine.open_session("p").unwrap();
    let child = SessionMeta {
        parent: Some("p".to_string()),
        ..SessionMeta::default()
    };
    assert!(
        engine
            .open_session_with_meta("c", MatrixStrategy::default(), child)
            .is_ok()
    );
}

#[test]
fn failure_mode_refs_round_trip() {
    let (engine, _d) = temp_engine();
    engine.open_session("s").unwrap();
    let mut fm = failure_mode("s", "fm-1", High, High);
    fm.refs = vec!["cpm:plan_1/t13".to_string()];
    let state = engine.add_failure_mode("s", fm).unwrap();
    assert_eq!(
        state.failure_modes[0].failure_mode.refs,
        vec!["cpm:plan_1/t13".to_string()]
    );
}

#[test]
fn list_sessions_returns_sorted_ids() {
    let (engine, _d) = temp_engine();
    engine.open_session("b").unwrap();
    engine.open_session("a").unwrap();
    assert_eq!(engine.list_sessions().unwrap(), vec!["a", "b"]);
}

#[test]
fn list_sessions_ignores_non_jsonl_files() {
    let (engine, dir) = temp_engine();
    engine.open_session("a").unwrap();
    std::fs::write(dir.path().join("README.md"), "readme").unwrap();
    std::fs::write(dir.path().join(".gitattributes"), "*.jsonl merge=union").unwrap();
    assert_eq!(engine.list_sessions().unwrap(), vec!["a"]);
}

#[test]
fn default_meta_is_not_serialized() {
    let event = Event::SessionOpened {
        session_id: "s".to_string(),
        matrix_strategy: MatrixStrategy::default(),
        meta: SessionMeta::default(),
    };
    assert!(!serde_json::to_string(&event).unwrap().contains("meta"));
}
