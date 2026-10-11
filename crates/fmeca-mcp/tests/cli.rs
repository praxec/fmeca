//! CLI skeleton tests: argument parsing and `--version` output.

use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use fmeca_mcp::cli::{Command, parse};

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn no_args_is_serve() {
    assert_eq!(parse(&[]).unwrap(), Command::Serve);
}

#[test]
fn version_flag_parses() {
    assert_eq!(parse(&args(&["--version"])).unwrap(), Command::Version);
}

#[test]
fn init_with_dir_parses() {
    assert_eq!(
        parse(&args(&["init", "--dir", "/tmp/project"])).unwrap(),
        Command::Init {
            dir: Some(PathBuf::from("/tmp/project")),
        }
    );
}

#[test]
fn unknown_flag_is_usage_error() {
    assert!(parse(&args(&["--bogus"])).is_err());
}

#[test]
fn version_flag_prints_package_version() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_fmeca-mcp"))
        .arg("--version")
        .output()
        .expect("run fmeca-mcp --version");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("fmeca-mcp {}\n", env!("CARGO_PKG_VERSION"))
    );
}
