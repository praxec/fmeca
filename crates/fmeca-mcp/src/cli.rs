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
        other => Err(UsageError(format!("unknown argument: {other}"))),
    }
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
