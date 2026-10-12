//! Loading project policies from `.fmeca/policies/<name>.toml`.

use std::fs;
use std::path::PathBuf;

use fmeca_policy::{PolicyError, PolicySource, load_project_policy};

const MINIMAL: &str = r#"
name = "minimal"
description = "Smallest valid policy"
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
dispositions = ["mitigate", "accept"]

[matrices.qualitative3x3]
derived_from = "qualitative3x3"
cells = [
  ["none", "none", "none"],
  ["none", "none", "none"],
  ["none", "none", "act"],
]

[signoff]
max_residual_band = "none"
"#;

/// A fresh, empty directory under cargo's per-target temp dir, unique per test.
fn fresh_dir(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("fmeca-policy-load")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// `<root>/policies/minimal.toml` holding `MINIMAL`; returns the policies dir.
fn policies_dir(test: &str) -> PathBuf {
    let dir = fresh_dir(test).join("policies");
    fs::create_dir_all(&dir).expect("create policies dir");
    fs::write(dir.join("minimal.toml"), MINIMAL).expect("write policy");
    dir
}

#[test]
fn valid_name_loads_project_policy() {
    let dir = policies_dir("valid_name");
    let loaded = load_project_policy(&dir, "minimal").expect("loads");
    assert_eq!(loaded.source, PolicySource::Project);
}

#[test]
fn name_with_slash_is_not_found() {
    let dir = policies_dir("slash");
    let outside = dir.parent().expect("parent").join("other");
    fs::create_dir_all(&outside).expect("create other dir");
    fs::write(outside.join("minimal.toml"), MINIMAL).expect("write policy");
    assert!(matches!(
        load_project_policy(&dir, "../other/minimal"),
        Err(PolicyError::NotFound(_))
    ));
}

#[test]
fn name_with_dotdot_is_not_found() {
    let dir = policies_dir("dotdot");
    fs::write(dir.parent().expect("parent").join("..toml"), MINIMAL).expect("write policy");
    assert!(matches!(
        load_project_policy(&dir, ".."),
        Err(PolicyError::NotFound(_))
    ));
}

#[test]
fn absent_file_is_not_found() {
    let dir = policies_dir("absent");
    assert!(matches!(
        load_project_policy(&dir, "standard"),
        Err(PolicyError::NotFound(_))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_escaping_dir_is_not_found() {
    let dir = policies_dir("symlink");
    let outside = dir.parent().expect("parent").join("outside.toml");
    fs::write(&outside, MINIMAL).expect("write policy");
    std::os::unix::fs::symlink(&outside, dir.join("escape.toml")).expect("symlink");
    assert!(matches!(
        load_project_policy(&dir, "escape"),
        Err(PolicyError::NotFound(_))
    ));
}
