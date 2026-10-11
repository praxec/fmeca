//! Project store tests: state-dir discovery, `init`, and `check`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use fmeca::{
    Domain, EntityRef, Event, EvidenceRef, FailureMode, FilesystemStore, MatrixStrategy,
    SessionMeta, StateStore,
};
use fmeca_mcp::project::{CheckLine, FileOutcome, check, init};
use fmeca_mcp::{PROJECT_DIR_NAME, StateDirSource, resolve_state_dir_from};

fn no_env(_key: &str) -> Option<OsString> {
    None
}

fn opened(session: &str) -> Event {
    Event::SessionOpened {
        session_id: session.to_string(),
        matrix_strategy: MatrixStrategy::default(),
        meta: SessionMeta::default(),
    }
}

fn failure_mode_added(session: &str, id: &str) -> Event {
    Event::FailureModeAdded {
        failure_mode: Box::new(FailureMode {
            id: id.to_string(),
            session_id: session.to_string(),
            component: EntityRef::new("comp:svc"),
            description: format!("failure mode {id}"),
            cause: Some("a cause".to_string()),
            effect: Some("an effect".to_string()),
            severity_observations: vec!["data_loss".to_string()],
            probability_observations: vec!["rare_edge_case".to_string()],
            domain: Domain::Runtime,
            scope: None,
            source: EvidenceRef::new("turn-1"),
            parent: None,
            refs: Vec::new(),
            local_effect: None,
            next_effect: None,
            single_point: false,
        }),
    }
}

fn append(dir: &Path, session: &str, event: &Event) {
    FilesystemStore::new(dir)
        .unwrap()
        .append(session, event)
        .unwrap();
}

#[test]
fn env_var_wins_over_project_dir() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(PROJECT_DIR_NAME)).unwrap();
    let get_env = |key: &str| (key == "FMECA_STATE_DIR").then(|| OsString::from("/env/state"));
    assert_eq!(
        resolve_state_dir_from(get_env, root.path()),
        (PathBuf::from("/env/state"), StateDirSource::Env,)
    );
}

#[test]
fn ancestor_fmeca_dir_is_found() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("a").join("b");
    fs::create_dir_all(root.path().join(PROJECT_DIR_NAME)).unwrap();
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        resolve_state_dir_from(no_env, &nested),
        (root.path().join(PROJECT_DIR_NAME), StateDirSource::Project,)
    );
}

#[test]
fn fmeca_regular_file_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(PROJECT_DIR_NAME), "not a directory").unwrap();
    let get_env = |key: &str| (key == "HOME").then(|| OsString::from("/home/user"));
    assert_eq!(
        resolve_state_dir_from(get_env, root.path()),
        (
            PathBuf::from("/home/user/.local/share/fmeca-mcp"),
            StateDirSource::User,
        )
    );
}

#[test]
fn user_default_when_no_project_dir() {
    let root = tempfile::tempdir().unwrap();
    let get_env = |key: &str| (key == "HOME").then(|| OsString::from("/home/user"));
    assert_eq!(
        resolve_state_dir_from(get_env, root.path()),
        (
            PathBuf::from("/home/user/.local/share/fmeca-mcp"),
            StateDirSource::User,
        )
    );
}

#[test]
fn init_creates_readme_and_gitattributes() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(PROJECT_DIR_NAME);
    assert_eq!(
        init(root.path()).unwrap(),
        vec![
            (dir.join("README.md"), FileOutcome::Created),
            (dir.join(".gitattributes"), FileOutcome::Created),
        ]
    );
}

#[test]
fn init_gitattributes_content_is_exact() {
    let root = tempfile::tempdir().unwrap();
    init(root.path()).unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join(PROJECT_DIR_NAME).join(".gitattributes")).unwrap(),
        "# fmeca analyses: append-only event logs. Union merge keeps both branches' events;\n\
         # run `fmeca-mcp check` after merging to catch duplicate ids.\n\
         *.jsonl text eol=lf merge=union\n"
    );
}

#[test]
fn init_readme_uses_hyphenated_session_example() {
    let root = tempfile::tempdir().unwrap();
    init(root.path()).unwrap();
    let readme = fs::read_to_string(root.path().join(PROJECT_DIR_NAME).join("README.md")).unwrap();
    assert!(readme.contains("checkout-technical-plan"));
}

#[test]
fn init_rerun_reports_unchanged() {
    let root = tempfile::tempdir().unwrap();
    init(root.path()).unwrap();
    let second = init(root.path()).unwrap();
    assert!(
        second
            .iter()
            .all(|(_, outcome)| *outcome == FileOutcome::Unchanged)
    );
}

#[test]
fn init_never_overwrites_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(PROJECT_DIR_NAME);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("README.md"), "my own notes").unwrap();
    init(root.path()).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        "my own notes"
    );
}

#[test]
fn check_reports_ok_for_valid_log() {
    let dir = tempfile::tempdir().unwrap();
    append(dir.path(), "svc-concept", &opened("svc-concept"));
    assert_eq!(
        check(dir.path()),
        vec![CheckLine {
            session_id: "svc-concept".to_string(),
            problems: Vec::new(),
        }]
    );
}

#[test]
fn check_reports_duplicate_failure_mode_id() {
    let dir = tempfile::tempdir().unwrap();
    append(dir.path(), "svc-concept", &opened("svc-concept"));
    append(
        dir.path(),
        "svc-concept",
        &failure_mode_added("svc-concept", "fm-1"),
    );
    append(
        dir.path(),
        "svc-concept",
        &failure_mode_added("svc-concept", "fm-1"),
    );
    let lines = check(dir.path());
    assert!(lines[0].problems[0].contains("line 3: duplicate_failure_mode_id"));
}

#[test]
fn check_ignores_non_jsonl_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "notes").unwrap();
    fs::write(dir.path().join(".gitattributes"), "*.jsonl merge=union").unwrap();
    assert_eq!(check(dir.path()), Vec::new());
}
