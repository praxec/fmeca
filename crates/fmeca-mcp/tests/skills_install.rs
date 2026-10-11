//! `fmeca-mcp skills install|uninstall` against a fixture skill set, in
//! throwaway directories, plus two binary-level checks. The behaviour tests do
//! not depend on the real `skills/` content.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use fmeca_mcp::skills::{
    BLOCK_BEGIN, BLOCK_END, EmbeddedSkill, Env, MANIFEST_FILE, execute, lock_path, parse,
    sha256_hex,
};
use tempfile::TempDir;

const DESCRIPTION: &str = "Concept stage of the fixture FMECA workflow";

/// What a `skills` run did: the exit code `fmeca-mcp` would return and stdout.
struct Output {
    code: Option<i32>,
    stdout: String,
}

fn skill(name: &str, description: &str, extra: &[&str]) -> EmbeddedSkill {
    let mut files = vec![(
        format!("{name}/SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
    )];
    for rel in extra {
        files.push((
            format!("{name}/{rel}"),
            format!("{{\"in\": \"{name}/{rel}\"}}\n"),
        ));
    }
    EmbeddedSkill {
        name: name.to_string(),
        description: description.to_string(),
        files,
    }
}

/// The reference skill (with an extra file) plus two `fmeca-*` skills.
fn fixture() -> Vec<EmbeddedSkill> {
    vec![
        skill("fmeca", "Shared FMECA reference", &["examples/x.json"]),
        skill("fmeca-concept", DESCRIPTION, &[]),
        skill("fmeca-vet", "Vet a finished analysis", &[]),
    ]
}

/// Every fixture file as (relative path, contents).
fn fixture_files() -> Vec<(String, String)> {
    fixture().into_iter().flat_map(|s| s.files).collect()
}

/// Run `skills <args>` with `home` as the home directory.
fn run(home: &Path, args: &[&str]) -> Output {
    run_with(&fixture(), home, args)
}

/// Lock files for every test live here, outside the temp project roots.
fn lock_dir() -> PathBuf {
    std::env::temp_dir().join("fmeca-mcp-skills-install-tests")
}

/// Run `skills <args>` over `skills` with `home` as the home directory.
fn run_with(skills: &[EmbeddedSkill], home: &Path, args: &[&str]) -> Output {
    run_waiting(skills, home, args, Duration::from_secs(30))
}

/// As [`run_with`], waiting at most `wait` for another run's lock.
fn run_waiting(skills: &[EmbeddedSkill], home: &Path, args: &[&str], wait: Duration) -> Output {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(_) => {
            return Output {
                code: Some(2),
                stdout: String::new(),
            };
        }
    };
    let env = Env {
        home: Some(home.to_path_buf()),
        exe: PathBuf::from(env!("CARGO_BIN_EXE_fmeca-mcp")),
        lock_dir: lock_dir(),
        lock_wait: wait,
    };
    let mut out = Vec::new();
    let code = match execute(&command, skills, &env, &mut out) {
        Ok(()) => 0,
        Err(_) => 1,
    };
    Output {
        code: Some(code),
        stdout: String::from_utf8_lossy(&out).into_owned(),
    }
}

fn project_cmd(dir: &Path, command: &str, target: &str, extra: &[&str]) -> Output {
    let dir_str = dir.to_str().expect("utf-8 temp path");
    let mut args = vec![command, "--target", target, "--project", dir_str];
    args.extend_from_slice(extra);
    run(dir, &args)
}

fn install(dir: &Path, target: &str, extra: &[&str]) -> Output {
    project_cmd(dir, "install", target, extra)
}

fn uninstall(dir: &Path, target: &str, extra: &[&str]) -> Output {
    project_cmd(dir, "uninstall", target, extra)
}

fn stdout(output: &Output) -> String {
    output.stdout.clone()
}

/// Paths of the fixture files that are missing under `root`.
fn missing_skill_files(root: &Path) -> Vec<String> {
    fixture_files()
        .into_iter()
        .map(|(rel, _)| rel)
        .filter(|rel| !root.join(rel).is_file())
        .collect()
}

/// Install `target` into a fresh project; (exit code, files missing under `roots`).
fn installed(target: &str, roots: &[&str]) -> (Option<i32>, Vec<String>) {
    let dir = TempDir::new().unwrap();
    let code = install(dir.path(), target, &[]).code;
    let missing = roots
        .iter()
        .flat_map(|r| missing_skill_files(&dir.path().join(r)))
        .collect();
    (code, missing)
}

fn ok_and_complete() -> (Option<i32>, Vec<String>) {
    (Some(0), Vec::new())
}

fn embedded(rel: &str) -> String {
    fixture_files()
        .into_iter()
        .find(|(p, _)| p == rel)
        .map(|(_, c)| c)
        .expect("fixture skill file")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("readable file")
}

fn agents_md(dir: &Path) -> PathBuf {
    dir.join("AGENTS.md")
}

/// `base` joined with a `/`-separated `rel`, using native separators, so it
/// compares equal to the paths the installer prints.
fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, c| p.join(c))
}

fn claude_concept_skill(dir: &Path) -> PathBuf {
    under(dir, ".claude/skills/fmeca-concept/SKILL.md")
}

/// The per-file lines of an install/uninstall run (before the summary).
fn file_lines(out: &str) -> Vec<&str> {
    out.lines()
        .take_while(|l| !l.starts_with("fmeca-mcp "))
        .collect()
}

/// Rewrite the manifest in `root` with `edit`.
fn edit_manifest(root: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = root.join(MANIFEST_FILE);
    let mut value: serde_json::Value = serde_json::from_str(&read(&path)).expect("manifest json");
    edit(&mut value);
    std::fs::write(&path, value.to_string()).expect("manifest written");
}

/// Install agents-md, then make AGENTS.md hold `prefix + old block + suffix`
/// as an older fmeca-mcp would have left it (manifest hash included).
fn agents_md_with_old_block(dir: &Path, prefix: &str, suffix: &str) {
    install(dir, "agents-md", &[]);
    let old_block = format!("{BLOCK_BEGIN}\nold fmeca-mcp notes\n{BLOCK_END}");
    std::fs::write(agents_md(dir), format!("{prefix}{old_block}{suffix}")).expect("write");
    let hash = sha256_hex(old_block.as_bytes());
    edit_manifest(&dir.join(".agents/skills"), |m| {
        m["agents_md_block"] = hash.into();
    });
}

/// Install agents-md over an AGENTS.md holding `text`; (exit code, AGENTS.md after).
fn install_over_agents_md(text: &str, extra: &[&str]) -> (Option<i32>, String) {
    let dir = TempDir::new().unwrap();
    std::fs::write(agents_md(dir.path()), text).unwrap();
    let code = install(dir.path(), "agents-md", extra).code;
    (code, read(&agents_md(dir.path())))
}

/// Lines of `text` that are exactly a begin marker.
fn begin_lines(text: &str) -> usize {
    text.lines().filter(|l| l.trim() == BLOCK_BEGIN).count()
}

fn gemini_table(dir: &Path, name: &str) -> toml::Table {
    let text = read(&dir.join(format!(".gemini/commands/{name}.toml")));
    toml::from_str(&text).expect("valid TOML")
}

// ------------------------------------------------------------ per target
// ------------------------------------------------------------ per target

#[test]
fn install_creates_skill_files_for_claude() {
    assert_eq!(installed("claude", &[".claude/skills"]), ok_and_complete());
}

#[test]
fn install_creates_skill_files_for_codex() {
    assert_eq!(installed("codex", &[".agents/skills"]), ok_and_complete());
}

#[test]
fn install_creates_skill_files_for_cursor() {
    assert_eq!(installed("cursor", &[".cursor/skills"]), ok_and_complete());
}

#[test]
fn install_creates_skill_files_for_copilot() {
    assert_eq!(installed("copilot", &[".github/skills"]), ok_and_complete());
}

#[test]
fn install_creates_skill_files_for_gemini() {
    assert_eq!(installed("gemini", &[".agents/skills"]), ok_and_complete());
}

#[test]
fn install_creates_skill_files_for_agents_md() {
    assert_eq!(
        installed("agents-md", &[".agents/skills"]),
        ok_and_complete()
    );
}

#[test]
fn install_creates_skill_files_for_all() {
    assert_eq!(
        installed("all", &[".claude/skills", ".agents/skills"]),
        ok_and_complete()
    );
}

#[test]
fn install_creates_skill_files_for_copilot_user_scope() {
    let home = TempDir::new().unwrap();
    let code = run(home.path(), &["install", "--target", "copilot", "--user"]).code;
    assert_eq!(
        (
            code,
            missing_skill_files(&home.path().join(".copilot/skills"))
        ),
        ok_and_complete()
    );
}

#[test]
fn install_creates_agents_md_block_for_agents_md() {
    let dir = TempDir::new().unwrap();
    let code = install(dir.path(), "agents-md", &[]).code;
    assert_eq!(
        (code, begin_lines(&read(&agents_md(dir.path())))),
        (Some(0), 1)
    );
}

#[test]
fn install_creates_one_command_per_fmeca_skill_for_gemini() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "gemini", &[]);
    let missing: Vec<&str> = ["fmeca-concept", "fmeca-vet"]
        .into_iter()
        .filter(|n| {
            !dir.path()
                .join(format!(".gemini/commands/{n}.toml"))
                .is_file()
        })
        .collect();
    assert_eq!(missing, Vec::<&str>::new());
}

#[test]
fn gemini_command_parses_with_the_skill_description() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "gemini", &[]);
    let table = gemini_table(dir.path(), "fmeca-concept");
    assert_eq!(
        table.get("description").and_then(|v| v.as_str()),
        Some(DESCRIPTION)
    );
}

#[test]
fn gemini_command_prompt_points_at_the_installed_skill() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "gemini", &[]);
    let table = gemini_table(dir.path(), "fmeca-concept");
    let prompt = table.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        prompt.contains("read the file .agents/skills/fmeca-concept/SKILL.md")
            && prompt.contains("{{args}}")
    );
}

#[test]
fn gemini_user_command_parses_with_a_spaced_home() {
    let tmp = TempDir::new().unwrap();
    // Only characters every OS allows; TOML quote/backslash escaping is unit-tested.
    let home = tmp.path().join("home with spaces & 'apostrophes'");
    std::fs::create_dir(&home).unwrap();
    run(&home, &["install", "--target", "gemini", "--user"]);
    let table = gemini_table(&home, "fmeca-vet");
    let skill = under(&home, ".agents/skills/fmeca-vet/SKILL.md");
    let prompt = table.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    assert!(prompt.contains(&skill.display().to_string()));
}

#[test]
fn install_for_all_writes_no_cursor_or_copilot_copies() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "all", &[]);
    assert!(!dir.path().join(".cursor").exists() && !dir.path().join(".github").exists());
}

#[test]
fn install_for_all_warns_cursor_and_copilot_may_list_twice() {
    let dir = TempDir::new().unwrap();
    let out = stdout(&install(dir.path(), "all", &[]));
    assert!(out.contains("may list each skill twice"));
}

#[test]
fn install_writes_a_manifest_with_every_file_hash() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let manifest: serde_json::Value = serde_json::from_str(&read(
        &dir.path().join(".claude/skills").join(MANIFEST_FILE),
    ))
    .unwrap();
    let expected: serde_json::Map<String, serde_json::Value> = fixture_files()
        .iter()
        .map(|(p, c)| ((*p).to_string(), sha256_hex(c.as_bytes()).into()))
        .collect();
    assert_eq!(manifest["files"], serde_json::Value::Object(expected));
}

#[test]
fn install_records_the_target_in_the_manifest() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "codex", &[]);
    install(dir.path(), "gemini", &[]);
    let manifest: serde_json::Value = serde_json::from_str(&read(
        &dir.path().join(".agents/skills").join(MANIFEST_FILE),
    ))
    .unwrap();
    assert_eq!(manifest["targets"], serde_json::json!(["codex", "gemini"]));
}

#[test]
fn lock_file_is_released_when_install_ends() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let root = dir.path().join(".claude/skills");
    let lock = std::fs::File::open(lock_path(&lock_dir(), &root)).unwrap();
    assert!(lock.try_lock().is_ok());
}

#[test]
fn lock_file_lives_outside_the_target_root() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let in_root: Vec<String> = std::fs::read_dir(dir.path().join(".claude/skills"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".lock"))
        .collect();
    assert_eq!(in_root, Vec::<String>::new());
}

#[test]
fn lock_file_survives_uninstall() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    uninstall(dir.path(), "claude", &[]);
    assert!(lock_path(&lock_dir(), &dir.path().join(".claude/skills")).is_file());
}

#[test]
fn install_writes_no_mcp_config() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "all", &[]);
    let written: Vec<_> = [
        ".mcp.json",
        ".cursor/mcp.json",
        ".vscode/mcp.json",
        ".codex",
        ".gemini",
    ]
    .into_iter()
    .filter(|p| dir.path().join(p).exists())
    .collect();
    assert_eq!(written, Vec::<&str>::new());
}

#[test]
fn install_prints_the_mcp_registration_command() {
    let dir = TempDir::new().unwrap();
    let out = stdout(&install(dir.path(), "claude", &[]));
    assert!(out.contains(
        "claude mcp add --transport stdio --scope project fmeca -- npx -y @matthew-cochran/fmeca"
    ));
}

#[test]
fn codex_project_install_prints_the_config_toml_snippet() {
    let dir = TempDir::new().unwrap();
    let out = stdout(&install(dir.path(), "codex", &[]));
    assert!(out.contains(
        "[mcp_servers.fmeca]\n  command = \"npx\"\n  args = [\"-y\", \"@matthew-cochran/fmeca\"]"
    ));
}

#[test]
fn gemini_install_prints_npx_and_binary_commands() {
    let dir = TempDir::new().unwrap();
    let out = stdout(&install(dir.path(), "gemini", &[]));
    // Canonicalise both sides: the printed path comes from current_exe(),
    // which may differ from CARGO_BIN_EXE_* in form (symlinks, 8.3 names,
    // `\\?\` prefixes) while naming the same file.
    let canonical = |p: &str| std::fs::canonicalize(p).ok();
    let exe = canonical(env!("CARGO_BIN_EXE_fmeca-mcp"));
    assert!(exe.is_some(), "test binary path must exist");
    let prefix = "gemini mcp add -s project fmeca ";
    let commands: Vec<&str> = out
        .lines()
        .filter_map(|l| l.trim().strip_prefix(prefix))
        .collect();
    let unquote = |c: &str| c.trim_matches(|ch| ch == '"' || ch == '\'').to_string();
    assert_eq!(
        (
            commands.contains(&"npx -y @matthew-cochran/fmeca"),
            commands.iter().any(|c| canonical(&unquote(c)) == exe)
        ),
        (true, true),
        "{out}"
    );
}

#[test]
fn all_project_install_prints_an_untyped_gemini_snippet() {
    let dir = TempDir::new().unwrap();
    let out = stdout(&install(dir.path(), "all", &[]));
    let gemini_snippet = out
        .lines()
        .skip_while(|l| l.trim() != "Gemini (.gemini/settings.json):")
        .nth(1)
        .map(str::trim)
        .map(str::to_string);
    assert_eq!(
        gemini_snippet.as_deref(),
        Some(
            r#"{"mcpServers":{"fmeca":{"command":"npx","args":["-y","@matthew-cochran/fmeca"]}}}"#
        )
    );
}

#[test]
fn second_install_reports_all_unchanged() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "gemini", &[]);
    let out = stdout(&install(dir.path(), "gemini", &[]));
    let statuses: BTreeSet<&str> = file_lines(&out)
        .into_iter()
        .filter_map(|l| l.split("  ").next())
        .collect();
    assert_eq!(statuses, BTreeSet::from(["unchanged"]));
}

#[test]
fn user_edited_file_is_kept_and_reported_modified() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let skill = claude_concept_skill(dir.path());
    std::fs::write(&skill, "my edit\n").unwrap();
    let output = install(dir.path(), "claude", &[]);
    let reported = output
        .stdout
        .contains(&format!("modified, skipped  {}", skill.display()));
    assert_eq!(
        (output.code, read(&skill), reported),
        (Some(0), "my edit\n".to_string(), true)
    );
}

#[test]
fn force_overwrites_modified_file() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let skill = claude_concept_skill(dir.path());
    std::fs::write(&skill, "my edit\n").unwrap();
    install(dir.path(), "claude", &["--force"]);
    assert_eq!(read(&skill), embedded("fmeca-concept/SKILL.md"));
}

#[test]
fn force_with_a_value_is_usage_error() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        install(dir.path(), "claude", &["--force=yes"]).code,
        Some(2)
    );
}

#[test]
fn dry_run_with_a_value_is_usage_error() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        install(dir.path(), "claude", &["--dry-run=no"]).code,
        Some(2)
    );
}

#[test]
fn foreign_file_is_skipped() {
    let dir = TempDir::new().unwrap();
    let skill = claude_concept_skill(dir.path());
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, "mine\n").unwrap();
    let out = stdout(&install(dir.path(), "claude", &[]));
    let reported = out.contains(&format!("foreign, skipped  {}", skill.display()));
    assert_eq!((read(&skill), reported), ("mine\n".to_string(), true));
}

#[test]
fn dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "agents-md", &["--dry-run"]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn install_handles_home_with_spaces() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home with spaces");
    std::fs::create_dir(&home).unwrap();
    run(&home, &["install", "--target", "claude", "--user"]);
    assert!(home.join(".claude/skills/fmeca-concept/SKILL.md").is_file());
}

#[test]
fn install_handles_project_dir_with_spaces() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("my project");
    std::fs::create_dir(&dir).unwrap();
    install(&dir, "cursor", &[]);
    assert!(dir.join(".cursor/skills/fmeca-concept/SKILL.md").is_file());
}

#[test]
fn upgrade_updates_files_matching_old_manifest() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let skill = claude_concept_skill(dir.path());
    let old = "fmeca-concept as fmeca-mcp 0.0.9 wrote it\n";
    std::fs::write(&skill, old).unwrap();
    edit_manifest(&dir.path().join(".claude/skills"), |m| {
        m["fmeca_mcp_version"] = "0.0.9".into();
        m["files"]["fmeca-concept/SKILL.md"] = sha256_hex(old.as_bytes()).into();
    });
    install(dir.path(), "claude", &[]);
    assert_eq!(read(&skill), embedded("fmeca-concept/SKILL.md"));
}

#[test]
fn upgrade_removes_files_the_new_version_no_longer_ships() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let retired = dir.path().join(".claude/skills/fmeca-retired/SKILL.md");
    std::fs::create_dir_all(retired.parent().unwrap()).unwrap();
    std::fs::write(&retired, "retired\n").unwrap();
    edit_manifest(&dir.path().join(".claude/skills"), |m| {
        m["files"]["fmeca-retired/SKILL.md"] = sha256_hex(b"retired\n").into();
    });
    install(dir.path(), "claude", &[]);
    assert!(!retired.exists());
}

// ------------------------------------------------------------ manifest safety

#[test]
fn manifest_with_parent_component_is_refused_and_deletes_nothing() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let victim = dir.path().join("victim.conf");
    std::fs::write(&victim, "precious\n").unwrap();
    edit_manifest(&dir.path().join(".claude/skills"), |m| {
        m["files"]["../../victim.conf"] = sha256_hex(b"precious\n").into();
    });
    let code = uninstall(dir.path(), "claude", &[]).code;
    assert_eq!(
        (
            code,
            victim.exists(),
            claude_concept_skill(dir.path()).exists()
        ),
        (Some(1), true, true)
    );
}

#[test]
fn manifest_with_backslash_key_is_refused() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    edit_manifest(&dir.path().join(".claude/skills"), |m| {
        m["files"]["fmeca-concept\\..\\..\\victim.conf"] = sha256_hex(b"x").into();
    });
    assert_eq!(install(dir.path(), "claude", &[]).code, Some(1));
}

#[cfg(unix)]
#[test]
fn symlink_out_of_the_target_root_is_refused() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude/skills")).unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        dir.path().join(".claude/skills/fmeca-concept"),
    )
    .unwrap();
    let output = install(dir.path(), "claude", &[]);
    assert_eq!(
        (
            output.code,
            std::fs::read_dir(outside.path()).unwrap().count()
        ),
        (Some(1), 0)
    );
}

#[cfg(unix)]
#[test]
fn symlinked_skill_file_is_refused_and_its_target_untouched() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("elsewhere.md");
    std::fs::write(&target, "outside\n").unwrap();
    let skill = claude_concept_skill(dir.path());
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &skill).unwrap();
    let code = install(dir.path(), "claude", &["--force"]).code;
    assert_eq!((code, read(&target)), (Some(1), "outside\n".to_string()));
}

#[test]
fn held_lock_makes_install_exit_1() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join(".claude/skills");
    std::fs::create_dir_all(lock_dir()).unwrap();
    let held = std::fs::File::create(lock_path(&lock_dir(), &root)).unwrap();
    held.lock().unwrap();
    // A short injected wait keeps the test fast.
    let path = dir.path().to_str().unwrap();
    let code = run_waiting(
        &fixture(),
        dir.path(),
        &["install", "--target", "claude", "--project", path],
        Duration::from_millis(200),
    )
    .code;
    drop(held);
    assert_eq!(code, Some(1));
}

#[test]
fn stale_lock_file_without_holder_does_not_block_install() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join(".claude/skills");
    std::fs::create_dir_all(lock_dir()).unwrap();
    std::fs::write(lock_path(&lock_dir(), &root), "left by a killed run\n").unwrap();
    assert_eq!(install(dir.path(), "claude", &[]).code, Some(0));
}

// ------------------------------------------------------------ AGENTS.md

#[test]
fn agents_md_content_outside_block_is_preserved() {
    let dir = TempDir::new().unwrap();
    let prefix = "# My agents\r\nkeep   this  \n\n";
    let suffix = "\n\n## Later notes\n\ttabbed, no final newline";
    agents_md_with_old_block(dir.path(), prefix, suffix);
    install(dir.path(), "agents-md", &[]);
    let text = read(&agents_md(dir.path()));
    let start = text.find(BLOCK_BEGIN).unwrap();
    let end = text.find(BLOCK_END).unwrap() + BLOCK_END.len();
    assert_eq!((&text[..start], &text[end..]), (prefix, suffix));
}

#[test]
fn agents_md_block_is_replaced_not_duplicated() {
    let dir = TempDir::new().unwrap();
    agents_md_with_old_block(dir.path(), "# Mine\n\n", "\n");
    install(dir.path(), "agents-md", &[]);
    let text = read(&agents_md(dir.path()));
    assert_eq!(
        (begin_lines(&text), text.contains("old fmeca-mcp notes")),
        (1, false)
    );
}

#[test]
fn agents_md_without_block_keeps_user_content_first() {
    let (_, text) = install_over_agents_md("# Mine\nno newline", &[]);
    assert!(text.starts_with(&format!("# Mine\nno newline\n\n{BLOCK_BEGIN}\n")));
}

#[test]
fn agents_md_nested_begin_is_refused_unchanged() {
    let text = format!("a\n{BLOCK_BEGIN}\n{BLOCK_BEGIN}\nx\n{BLOCK_END}\n");
    assert_eq!(install_over_agents_md(&text, &[]), (Some(1), text));
}

#[test]
fn agents_md_duplicate_block_is_refused_unchanged() {
    let text = format!("{BLOCK_BEGIN}\nx\n{BLOCK_END}\n{BLOCK_BEGIN}\ny\n{BLOCK_END}\n");
    assert_eq!(install_over_agents_md(&text, &[]), (Some(1), text));
}

#[test]
fn agents_md_stray_end_is_refused_unchanged() {
    let text = format!("notes\n{BLOCK_END}\n");
    assert_eq!(install_over_agents_md(&text, &[]), (Some(1), text));
}

#[test]
fn agents_md_begin_without_end_is_refused_unchanged() {
    let text = format!("notes\n{BLOCK_BEGIN}\nrest\n");
    assert_eq!(install_over_agents_md(&text, &[]), (Some(1), text));
}

#[test]
fn agents_md_refusal_creates_no_skills_dir() {
    let dir = TempDir::new().unwrap();
    std::fs::write(agents_md(dir.path()), format!("{BLOCK_END}\n")).unwrap();
    install(dir.path(), "agents-md", &[]);
    assert!(!dir.path().join(".agents").exists());
}

#[test]
fn agents_md_markers_in_prose_are_not_matched() {
    let original = format!("Wrap notes in `{BLOCK_BEGIN}` and `{BLOCK_END}` lines.\n");
    let (code, text) = install_over_agents_md(&original, &[]);
    assert_eq!(
        (code, text.starts_with(&original), begin_lines(&text)),
        (Some(0), true, 1)
    );
}

#[test]
fn agents_md_force_does_not_corrupt_nested_markers() {
    let text = format!("{BLOCK_BEGIN}\n{BLOCK_BEGIN}\n{BLOCK_END}\n");
    assert_eq!(install_over_agents_md(&text, &["--force"]), (Some(1), text));
}

#[test]
fn agents_md_user_edited_block_is_kept() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "agents-md", &[]);
    let edited = format!("# Mine\n\n{BLOCK_BEGIN}\nmy own words\n{BLOCK_END}\n");
    std::fs::write(agents_md(dir.path()), &edited).unwrap();
    install(dir.path(), "agents-md", &[]);
    assert_eq!(read(&agents_md(dir.path())), edited);
}

#[test]
fn agents_md_force_replaces_an_edited_block() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "agents-md", &[]);
    std::fs::write(
        agents_md(dir.path()),
        format!("# Mine\n\n{BLOCK_BEGIN}\nmy own words\n{BLOCK_END}\n"),
    )
    .unwrap();
    install(dir.path(), "agents-md", &["--force"]);
    let text = read(&agents_md(dir.path()));
    assert_eq!(
        (
            begin_lines(&text),
            text.contains("my own words"),
            text.starts_with("# Mine\n\n")
        ),
        (1, false, true)
    );
}

#[test]
fn agents_md_crlf_file_gets_a_crlf_block() {
    let (_, text) = install_over_agents_md("# Mine\r\nline\r\n", &[]);
    assert!(!text.replace("\r\n", "").contains('\n'));
}

#[test]
fn agents_md_install_then_uninstall_is_byte_identical() {
    let originals = [
        "# Mine\n",
        "# Mine",
        "# Mine\r\nmore\r\n",
        "# Mine\r\nno final newline",
        "",
        "a\n\n",
        "mixed\r\nends lf\n",
    ];
    let changed: Vec<&str> = originals
        .into_iter()
        .filter(|original| {
            let dir = TempDir::new().unwrap();
            std::fs::write(agents_md(dir.path()), original).unwrap();
            install(dir.path(), "agents-md", &[]);
            uninstall(dir.path(), "agents-md", &[]);
            std::fs::read(agents_md(dir.path())).ok() != Some(original.as_bytes().to_vec())
        })
        .collect();
    assert_eq!(changed, Vec::<&str>::new());
}

#[test]
fn agents_md_created_by_install_is_deleted_by_uninstall() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "agents-md", &[]);
    uninstall(dir.path(), "agents-md", &[]);
    assert!(!agents_md(dir.path()).exists());
}

#[cfg(unix)]
#[test]
fn agents_md_symlink_inside_the_project_is_written_through() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("CLAUDE.md"), "# Claude\n").unwrap();
    std::os::unix::fs::symlink("CLAUDE.md", agents_md(dir.path())).unwrap();
    install(dir.path(), "agents-md", &[]);
    let still_link = std::fs::symlink_metadata(agents_md(dir.path()))
        .unwrap()
        .file_type()
        .is_symlink();
    assert_eq!(
        (
            still_link,
            begin_lines(&read(&dir.path().join("CLAUDE.md")))
        ),
        (true, 1)
    );
}

#[cfg(unix)]
#[test]
fn agents_md_symlink_outside_the_project_is_refused() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("AGENTS.md");
    std::fs::write(&target, "# Elsewhere\n").unwrap();
    std::os::unix::fs::symlink(&target, agents_md(dir.path())).unwrap();
    let code = install(dir.path(), "agents-md", &[]).code;
    assert_eq!(
        (code, read(&target)),
        (Some(1), "# Elsewhere\n".to_string())
    );
}

#[test]
fn agents_md_with_user_scope_is_usage_error() {
    let home = TempDir::new().unwrap();
    let output = run(home.path(), &["install", "--target", "agents-md", "--user"]);
    assert_eq!(output.code, Some(2));
}

// ------------------------------------------------------------ uninstall

#[test]
fn uninstall_keeps_user_modified_files() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let edited = claude_concept_skill(dir.path());
    std::fs::write(&edited, "my edit\n").unwrap();
    uninstall(dir.path(), "claude", &[]);
    let untouched = dir.path().join(".claude/skills/fmeca-vet/SKILL.md");
    assert_eq!((edited.is_file(), untouched.exists()), (true, false));
}

#[test]
fn uninstall_keeps_foreign_files() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    let mine = dir.path().join(".claude/skills/my-skill/SKILL.md");
    std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
    std::fs::write(&mine, "mine\n").unwrap();
    uninstall(dir.path(), "claude", &[]);
    assert!(mine.is_file());
}

#[test]
fn uninstall_dry_run_removes_nothing() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "codex", &[]);
    uninstall(dir.path(), "codex", &["--dry-run"]);
    assert_eq!(
        missing_skill_files(&dir.path().join(".agents/skills")),
        Vec::<String>::new()
    );
}

#[test]
fn uninstall_dry_run_prints_no_shared_root_note() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "codex", &[]);
    let out = stdout(&uninstall(dir.path(), "codex", &["--dry-run"]));
    assert!(!out.contains("Note:"));
}

#[test]
fn uninstall_removes_the_agents_md_block_and_keeps_the_rest() {
    let dir = TempDir::new().unwrap();
    std::fs::write(agents_md(dir.path()), "# Mine\n").unwrap();
    install(dir.path(), "agents-md", &[]);
    uninstall(dir.path(), "agents-md", &[]);
    assert_eq!(read(&agents_md(dir.path())), "# Mine\n");
}

#[test]
fn uninstall_one_target_keeps_shared_root_for_other_target() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "codex", &[]);
    install(dir.path(), "agents-md", &[]);
    let out = stdout(&uninstall(dir.path(), "codex", &[]));
    assert_eq!(
        (
            missing_skill_files(&dir.path().join(".agents/skills")),
            out.contains("kept (still used by agents-md)")
        ),
        (Vec::<String>::new(), true)
    );
}

#[test]
fn uninstall_gemini_keeps_agents_skills_for_codex_and_removes_commands() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "codex", &[]);
    install(dir.path(), "gemini", &[]);
    uninstall(dir.path(), "gemini", &[]);
    assert_eq!(
        (
            missing_skill_files(&dir.path().join(".agents/skills")),
            dir.path().join(".gemini/commands").exists()
        ),
        (Vec::<String>::new(), false)
    );
}

#[test]
fn uninstall_all_keeps_separate_claude_install() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "claude", &[]);
    install(dir.path(), "all", &[]);
    uninstall(dir.path(), "all", &[]);
    assert_eq!(
        (
            missing_skill_files(&dir.path().join(".claude/skills")),
            dir.path().join(".agents/skills").exists()
        ),
        (Vec::<String>::new(), false)
    );
}

#[test]
fn last_target_uninstall_removes_files() {
    let dir = TempDir::new().unwrap();
    install(dir.path(), "codex", &[]);
    install(dir.path(), "gemini", &[]);
    uninstall(dir.path(), "codex", &[]);
    uninstall(dir.path(), "gemini", &[]);
    assert!(!dir.path().join(".agents").join("skills").exists());
}

// ------------------------------------------------------------ CLI

#[test]
fn unknown_target_is_usage_error() {
    let dir = TempDir::new().unwrap();
    assert_eq!(install(dir.path(), "emacs", &[]).code, Some(2));
}

#[test]
fn install_without_scope_is_usage_error() {
    let home = TempDir::new().unwrap();
    let output = run(home.path(), &["install", "--target", "claude"]);
    assert_eq!(output.code, Some(2));
}

// ------------------------------------------------------------ concurrency

#[test]
fn concurrent_installs_are_serialised_by_the_file_lock() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    let codes: Vec<Option<i32>> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || install(&path, "claude", &[]).code)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        (
            codes,
            missing_skill_files(&dir.path().join(".claude/skills"))
        ),
        (vec![Some(0); 4], Vec::<String>::new())
    );
}

// ------------------------------------------------------------ binary

#[test]
fn embedded_skill_set_builds() {
    let project = TempDir::new().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_fmeca-mcp"))
        .args(["skills", "install", "--target", "claude", "--project"])
        .arg(project.path())
        .arg("--dry-run")
        .status()
        .expect("the binary runs");
    assert_eq!(status.code(), Some(0));
}

#[test]
fn help_flag_lists_the_skills_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_fmeca-mcp"))
        .arg("--help")
        .output()
        .expect("the binary runs");
    assert!(String::from_utf8_lossy(&output.stdout).contains("fmeca-mcp skills install"));
}

#[test]
fn copilot_project_install_prints_the_dot_mcp_json_snippet() {
    let dir = TempDir::new().unwrap();
    let out = stdout(&install(dir.path(), "copilot", &[]));
    assert!(out.contains(
        "Add to .mcp.json:\n  {\"mcpServers\":{\"fmeca\":{\"type\":\"stdio\",\"command\":\"npx\""
    ));
}

#[test]
fn copilot_user_install_prints_the_portable_user_config_path() {
    let home = TempDir::new().unwrap();
    let out = stdout(&run(
        home.path(),
        &["install", "--target", "copilot", "--user"],
    ));
    assert!(out.contains("~/.copilot/mcp-config.json"));
}

#[test]
fn skill_file_path_escaping_its_skill_is_refused_before_any_write() {
    let dir = TempDir::new().unwrap();
    let mut skills = fixture();
    skills[1]
        .files
        .push(("fmeca-concept/../../escape.md".to_string(), "x".to_string()));
    let path = dir.path().to_str().unwrap();
    let code = run_with(
        &skills,
        dir.path(),
        &["install", "--target", "claude", "--project", path],
    )
    .code;
    assert_eq!(
        (code, std::fs::read_dir(dir.path()).unwrap().count()),
        (Some(1), 0)
    );
}

#[cfg(unix)]
#[test]
fn project_scope_refuses_symlinked_root_outside_project() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".claude")).unwrap();
    let code = install(dir.path(), "claude", &[]).code;
    assert_eq!(
        (code, std::fs::read_dir(outside.path()).unwrap().count()),
        (Some(1), 0)
    );
}

#[cfg(unix)]
#[test]
fn targets_sharing_a_canonical_root_take_one_lock() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".claude")).unwrap();
    std::os::unix::fs::symlink(".claude", dir.path().join(".agents")).unwrap();
    let path = dir.path().to_str().unwrap();
    let started = std::time::Instant::now();
    let code = run_waiting(
        &fixture(),
        dir.path(),
        &["install", "--target", "all", "--project", path],
        Duration::from_secs(5),
    )
    .code;
    assert_eq!(
        (code, started.elapsed() < Duration::from_secs(3)),
        (Some(0), true)
    );
}

#[test]
fn spellings_of_one_root_and_a_missing_root_share_a_lock_path() {
    let dir = TempDir::new().unwrap();
    let plain = dir.path().join("proj");
    std::fs::create_dir(&plain).unwrap();
    let dotted = plain.join(".").join(".claude").join("skills");
    let missing = plain.join(".claude").join("skills");
    let lock = lock_dir();
    std::fs::create_dir_all(&missing).unwrap();
    let existing = lock_path(&lock, &missing);
    std::fs::remove_dir_all(plain.join(".claude")).unwrap();
    assert_eq!(
        (lock_path(&lock, &dotted), lock_path(&lock, &missing)),
        (existing.clone(), existing)
    );
}
