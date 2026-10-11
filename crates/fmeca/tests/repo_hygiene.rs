use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "ico", "jpg", "gif", "db", "sqlite", "zip", "gz", "tgz",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn is_binary_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            BINARY_EXTENSIONS
                .iter()
                .any(|b| ext.eq_ignore_ascii_case(b))
        })
}

fn contains_crlf(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| bytes.windows(2).any(|w| w == b"\r\n"))
}

#[test]
fn tracked_text_files_are_lf() {
    let root = workspace_root();
    let output = match Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&root)
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return, // packaged crate or no git: nothing to check
    };
    let tracked = String::from_utf8_lossy(&output.stdout);
    let offenders: Vec<&str> = tracked
        .split('\0')
        .filter(|p| !p.is_empty() && !is_binary_path(p))
        .filter(|p| contains_crlf(&root.join(p)))
        .collect();
    assert!(
        offenders.is_empty(),
        "tracked text files contain CRLF: {offenders:?}"
    );
}
