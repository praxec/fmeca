//! Command-line parsing for `fmeca-mcp`.
//!
//! The binary serves MCP over stdio by default; `--version`, `--help`, `init`,
//! `check` and `skills` are the only other invocations. Parsing is pure (arguments in,
//! [`Command`] out) so it is tested without spawning a process.

use std::path::PathBuf;

use crate::skills::{self, SkillsCommand};

/// A parsed invocation. `Serve` (no arguments) is the default and runs the
/// existing MCP server unchanged.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Serve MCP over stdio.
    Serve,
    /// Print `fmeca-mcp <version>`.
    Version,
    /// Print [`USAGE`].
    Help,
    /// Scaffold the project state dir.
    Init { dir: Option<PathBuf> },
    /// Validate every log in a state dir.
    Check { dir: Option<PathBuf> },
    /// Install or uninstall the embedded agent skills.
    Skills(SkillsCommand),
    /// Suggest policy-relative actions for a session or an analysis file.
    Suggest(SuggestArgs),
    /// `policy validate` / `policy list`.
    Policy(PolicyCommand),
}

/// Output format for `suggest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Machine-readable JSON (the default).
    Json,
    /// Human-readable Markdown.
    Markdown,
}

/// A parsed `suggest` invocation.
#[derive(Debug, PartialEq, Eq)]
pub struct SuggestArgs {
    /// Exactly one of `session_id` / `analysis` is set.
    pub session_id: Option<String>,
    /// A `{matrix_strategy?, failure_modes:[...]}` JSON file.
    pub analysis: Option<PathBuf>,
    /// Policy preset name or project file stem; defaults to `compat-0.1`.
    pub policy: Option<String>,
    /// `YYYY-MM-DD`; defaults to today (UTC) in the CLI.
    pub as_of: Option<String>,
    /// Requested output format.
    pub format: OutputFormat,
    /// State dir; resolved like the server when absent.
    pub dir: Option<PathBuf>,
}

/// A parsed `policy` subcommand.
#[derive(Debug, PartialEq, Eq)]
pub enum PolicyCommand {
    /// Validate one policy file and print `ok <name> <hash>`.
    Validate {
        /// The TOML policy file.
        file: PathBuf,
    },
    /// List built-in presets and project policies.
    List {
        /// State dir; resolved like the server when absent.
        dir: Option<PathBuf>,
    },
}

/// A user error, printed before [`USAGE`] when parsing fails.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

/// Parse `args` (without the program name).
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let Some(first) = args.first() else {
        return Ok(Command::Serve);
    };

    match first.as_str() {
        "--version" | "-V" => single(args, Command::Version),
        "--help" | "-h" => single(args, Command::Help),
        "init" => parse_with_dir(&args[1..]).map(|dir| Command::Init { dir }),
        "check" => parse_with_dir(&args[1..]).map(|dir| Command::Check { dir }),
        "skills" => skills::parse(&args[1..])
            .map(Command::Skills)
            .map_err(UsageError),
        "suggest" => parse_suggest(&args[1..]).map(Command::Suggest),
        "policy" => parse_policy(&args[1..]).map(Command::Policy),
        other => Err(UsageError(format!("unknown argument: {other}"))),
    }
}

/// Parse `suggest (<session-id> | --analysis <file>) [--policy <name>]
/// [--as-of YYYY-MM-DD] [--format json|markdown] [--dir <state-dir>]`.
fn parse_suggest(rest: &[String]) -> Result<SuggestArgs, UsageError> {
    let mut args = SuggestArgs {
        session_id: None,
        analysis: None,
        policy: None,
        as_of: None,
        format: OutputFormat::Json,
        dir: None,
    };
    let mut format_seen = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--analysis" => {
                let value = next_value(rest, &mut index, "--analysis")?;
                if args.analysis.is_some() {
                    return Err(UsageError("--analysis given twice".to_string()));
                }
                args.analysis = Some(PathBuf::from(value));
            }
            "--policy" => {
                let value = next_value(rest, &mut index, "--policy")?;
                if args.policy.is_some() {
                    return Err(UsageError("--policy given twice".to_string()));
                }
                args.policy = Some(value);
            }
            "--as-of" => {
                let value = next_value(rest, &mut index, "--as-of")?;
                if args.as_of.is_some() {
                    return Err(UsageError("--as-of given twice".to_string()));
                }
                args.as_of = Some(value);
            }
            "--format" => {
                let value = next_value(rest, &mut index, "--format")?;
                if format_seen {
                    return Err(UsageError("--format given twice".to_string()));
                }
                format_seen = true;
                args.format = match value.as_str() {
                    "json" => OutputFormat::Json,
                    "markdown" => OutputFormat::Markdown,
                    other => return Err(UsageError(format!("unknown --format: {other}"))),
                };
            }
            "--dir" => {
                let value = next_value(rest, &mut index, "--dir")?;
                if args.dir.is_some() {
                    return Err(UsageError("--dir given twice".to_string()));
                }
                args.dir = Some(PathBuf::from(value));
            }
            other if other.starts_with("--") => {
                return Err(UsageError(format!("unknown argument: {other}")));
            }
            other => {
                if args.session_id.is_some() {
                    return Err(UsageError(format!("unexpected argument: {other}")));
                }
                args.session_id = Some(other.to_string());
            }
        }
        index += 1;
    }
    match (&args.session_id, &args.analysis) {
        (Some(_), Some(_)) => Err(UsageError(
            "suggest accepts a session id or --analysis, not both".to_string(),
        )),
        (None, None) => Err(UsageError(
            "suggest needs a session id or --analysis".to_string(),
        )),
        _ => Ok(args),
    }
}

/// Parse `policy validate <file>` and `policy list [--dir <state-dir>]`.
fn parse_policy(rest: &[String]) -> Result<PolicyCommand, UsageError> {
    match rest.first().map(String::as_str) {
        Some("validate") => match &rest[1..] {
            [file] => Ok(PolicyCommand::Validate {
                file: PathBuf::from(file),
            }),
            [] => Err(UsageError("policy validate requires a file".to_string())),
            [other, ..] => Err(UsageError(format!("unexpected argument: {other}"))),
        },
        Some("list") => match &rest[1..] {
            [] => Ok(PolicyCommand::List { dir: None }),
            [flag, dir] if flag == "--dir" => Ok(PolicyCommand::List {
                dir: Some(PathBuf::from(dir)),
            }),
            [flag] if flag == "--dir" => Err(UsageError("--dir requires a value".to_string())),
            [other, ..] => Err(UsageError(format!("unknown argument: {other}"))),
        },
        Some(other) => Err(UsageError(format!("unknown policy subcommand: {other}"))),
        None => Err(UsageError("policy requires a subcommand".to_string())),
    }
}

/// The value following a flag, advancing `index`.
fn next_value(rest: &[String], index: &mut usize, flag: &str) -> Result<String, UsageError> {
    *index += 1;
    rest.get(*index)
        .cloned()
        .ok_or_else(|| UsageError(format!("{flag} requires a value")))
}

/// Accept a flag only when it is the sole argument.
fn single(args: &[String], command: Command) -> Result<Command, UsageError> {
    if args.len() == 1 {
        Ok(command)
    } else {
        Err(UsageError(format!(
            "unexpected arguments after {}",
            args[0]
        )))
    }
}

/// Parse the optional `--dir <path>` pair for `init` and `check`.
fn parse_with_dir(rest: &[String]) -> Result<Option<PathBuf>, UsageError> {
    match rest {
        [] => Ok(None),
        [flag, dir] if flag == "--dir" => Ok(Some(PathBuf::from(dir))),
        [flag] if flag == "--dir" => Err(UsageError("--dir requires a path".to_string())),
        [other, ..] => Err(UsageError(format!("unknown argument: {other}"))),
    }
}

/// Help text. `serve` is the default and is not spelled as a subcommand.
pub const USAGE: &str = "\
fmeca-mcp — stdio MCP server for structured FMECA

USAGE:
    fmeca-mcp                 Serve MCP over stdio (default)
    fmeca-mcp --version, -V   Print the version
    fmeca-mcp --help, -h      Print this help
    fmeca-mcp init [--dir D]  Create .fmeca/README.md and .fmeca/.gitattributes
    fmeca-mcp check [--dir D] Validate every <session_id>.jsonl log in the state dir
    fmeca-mcp skills install|uninstall ...
                              Install or remove the agent skills (see below)
    fmeca-mcp suggest (<session-id> | --analysis F) [--policy P] [--as-of D]
                      [--format json|markdown] [--dir D]
                              Suggest policy actions; exit 0 pass, 1 fail, 2 error
    fmeca-mcp policy validate <file>
                              Print `ok <name> <hash>`, or the error (exit 2);
                              `name` must equal the file stem, not a preset name
    fmeca-mcp policy list [--dir D]
                              Print `<source> <name> <hash> <class>` per policy;
                              a project file that fails to load prints
                              `project <stem> - - <ERROR>`

The state dir is FMECA_STATE_DIR, else the nearest ancestor with a .fmeca/
directory, else $XDG_DATA_HOME/fmeca-mcp or $HOME/.local/share/fmeca-mcp,
else ./fmeca-state.

SKILLS:
    fmeca-mcp skills install   --target <target> (--project <dir> | --user) [--dry-run] [--force]
    fmeca-mcp skills uninstall --target <target> (--project <dir> | --user) [--dry-run]

Targets:
  claude     Claude Code      .claude/skills/          (user: ~/.claude/skills/)
  codex      OpenAI Codex     .agents/skills/          (user: ~/.agents/skills/)
  cursor     Cursor           .cursor/skills/          (user: ~/.cursor/skills/)
  copilot    GitHub Copilot   .github/skills/          (user: ~/.copilot/skills/)
  gemini     Gemini CLI       .agents/skills/ + .gemini/commands/fmeca-*.toml
                              (user: ~/.agents/skills/ + ~/.gemini/commands/fmeca-*.toml)
  agents-md  AGENTS.md block + .agents/skills/         (project only; for user scope use codex)
  all        .claude/skills/ + .agents/skills/

A file you edited, or one fmeca-mcp did not write, is kept and reported as
skipped; --force overwrites it. No MCP configuration is written: the command
to register the server is printed after the install.
";
