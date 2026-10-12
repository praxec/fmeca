//! `fmeca-mcp` — standalone stdio MCP server for the deterministic, offline
//! structured-FMECA kernel.
//!
//! Run from source:
//!
//! ```bash
//! cargo run -p fmeca-mcp
//! ```
//!
//! After install the binary is on your PATH as `fmeca-mcp`. It speaks MCP over
//! stdio (the standard transport for Claude Code, Cursor, and most MCP clients).
//! The on-disk event log lives under the resolved state dir (see
//! [`fmeca_mcp::resolve_state_dir`]); override with `FMECA_STATE_DIR`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::cli::{Command, OutputFormat, PolicyCommand, SuggestArgs, USAGE, parse};
use fmeca_mcp::project::{FileOutcome, check, init};
use fmeca_mcp::skills;
use fmeca_mcp::{FmecaServer, resolve_state_dir, resolve_state_dir_from};
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{}", error.0);
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    match command {
        Command::Serve => serve(),
        Command::Version => {
            println!("fmeca-mcp {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Command::Init { dir } => run_init(dir),
        Command::Check { dir } => run_check(dir),
        Command::Skills(command) => {
            // The installer needs no async runtime and sets its own exit code.
            if skills::run(command) != ExitCode::SUCCESS {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Suggest(args) => run_suggest(args),
        Command::Policy(PolicyCommand::Validate { file }) => run_policy_validate(&file),
        Command::Policy(PolicyCommand::List { dir }) => run_policy_list(dir),
    }
}

/// One-shot analysis file shape, mirroring the `analyze` MCP arguments.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalysisFile {
    #[serde(default)]
    matrix_strategy: fmeca::MatrixStrategy,
    failure_modes: Vec<fmeca::AnalyzeInput>,
}

/// Print `message` to stderr and exit 2 (usage or policy error).
fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

/// `suggest`: evaluate a session or an analysis file under a policy, print the
/// result, and exit 0 (pass) / 1 (fail) / 2 (usage or policy error).
fn run_suggest(args: SuggestArgs) -> anyhow::Result<()> {
    let state_dir = args.dir.clone().unwrap_or_else(resolve_state_dir);
    let project_dir = policy_project_dir(&state_dir);
    let policy_name = args.policy.as_deref().unwrap_or("compat-0.1");
    let loaded = match fmeca_policy::resolve(policy_name, project_dir.as_deref()) {
        Ok(loaded) => loaded,
        Err(error) => fail(error),
    };
    let as_of = match args.as_of.as_deref() {
        Some(text) => match fmeca_policy::parse_date(text) {
            Ok(date) => date,
            Err(error) => fail(error),
        },
        None => today_utc(),
    };
    let subject = match build_subject(&args, &state_dir) {
        Ok(subject) => subject,
        Err(error) => fail(error),
    };
    let suggestions = match fmeca_policy::evaluate(&subject, &loaded, as_of) {
        Ok(suggestions) => suggestions,
        Err(error) => fail(error),
    };
    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&suggestions)?),
        OutputFormat::Markdown => {
            print!("{}", fmeca_policy::render::to_markdown(&suggestions));
        }
    }
    if suggestions.signoff.pass {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

/// Build the evaluator's [`fmeca_policy::Subject`] from the session or file.
fn build_subject(args: &SuggestArgs, state_dir: &Path) -> Result<fmeca_policy::Subject, String> {
    if let Some(session_id) = &args.session_id {
        let store = FilesystemStore::new(state_dir).map_err(|error| error.to_string())?;
        let engine = Engine::new(Arc::new(store));
        let state = engine
            .state(session_id)
            .map_err(|error| error.to_string())?;
        Ok(fmeca_policy::subject_from_state(&state))
    } else if let Some(path) = &args.analysis {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let file: AnalysisFile =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        fmeca_policy::subject_from_analysis(file.matrix_strategy, &file.failure_modes)
            .map_err(|error| error.to_string())
    } else {
        Err("suggest needs a session id or --analysis".to_string())
    }
}

/// Today's UTC date, as `YYYY-MM-DD`. The only clock read; `fmeca-policy`
/// itself stays clock-free.
fn today_utc() -> fmeca_policy::Date {
    let now = time::OffsetDateTime::now_utc();
    let date = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    );
    match fmeca_policy::parse_date(&date) {
        Ok(date) => date,
        Err(error) => fail(error),
    }
}

/// The project policies dir (`<state_dir>/policies`) when the state dir is a
/// `.fmeca/` directory, mirroring the MCP server.
fn policy_project_dir(state_dir: &Path) -> Option<PathBuf> {
    (state_dir.file_name().and_then(|name| name.to_str()) == Some(".fmeca"))
        .then(|| state_dir.join("policies"))
}

/// `policy validate <file>`: print `ok <name> <hash>` or fail with exit 2.
fn run_policy_validate(file: &Path) -> anyhow::Result<()> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) => fail(format!("INVALID_POLICY: {}: {error}", file.display())),
    };
    let policy = match fmeca_policy::parse_policy(&text) {
        Ok(policy) => policy,
        Err(error) => fail(error),
    };
    if let Err(error) = fmeca_policy::validate(&policy) {
        fail(error);
    }
    println!("ok {} {}", policy.name, fmeca_policy::policy_hash(&policy));
    Ok(())
}

/// `policy list [--dir <state-dir>]`: `<source> <name> <hash> <class>` lines.
fn run_policy_list(dir: Option<PathBuf>) -> anyhow::Result<()> {
    for name in fmeca_policy::PRESET_NAMES {
        if let Some(loaded) = fmeca_policy::preset(name) {
            print_policy_row("preset", &loaded);
        }
    }
    let state_dir = dir.unwrap_or_else(resolve_state_dir);
    if let Some(policies) = policy_project_dir(&state_dir) {
        let mut names: Vec<String> = std::fs::read_dir(&policies)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| policy_stem(&entry.path()))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names.dedup();
        for name in names {
            match fmeca_policy::load_project_policy(&policies, &name) {
                Ok(loaded) => print_policy_row("project", &loaded),
                Err(error) => eprintln!("{error}"),
            }
        }
    }
    Ok(())
}

fn print_policy_row(source: &str, loaded: &fmeca_policy::LoadedPolicy) {
    println!(
        "{source} {} {} {}",
        loaded.policy.name, loaded.hash, loaded.policy.class
    );
}

/// A policy file stem, when `path` is a `<valid-name>.toml`.
fn policy_stem(path: &Path) -> Option<String> {
    let stem = path.file_stem().and_then(|stem| stem.to_str())?;
    let is_toml = path.extension().and_then(|ext| ext.to_str()) == Some("toml");
    (is_toml && fmeca_policy::valid_policy_name(stem)).then(|| stem.to_string())
}

/// Serve MCP over stdio using the resolved state dir. This is the no-argument
/// path and runs the existing server unchanged.
fn serve() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (state_dir, source) = resolve_state_dir_from(|key| std::env::var_os(key), &cwd);
    tracing::info!(?state_dir, source = ?source, "starting fmeca-mcp stdio server");
    let store = FilesystemStore::new(&state_dir)?;
    let engine = Engine::new(Arc::new(store));
    let server = with_jev(FmecaServer::new(engine).with_state_dir(state_dir));
    tokio::runtime::Runtime::new()?.block_on(server.serve_stdio())
}

/// Give the server a Jev judge when `OPENROUTER_API_KEY` is set. A rejected
/// configuration disables `vet` and is reported by it as `bad_config`.
#[cfg(feature = "jev")]
fn with_jev(server: FmecaServer) -> FmecaServer {
    use fmeca_mcp::jev::{JevConfig, JudgmentModel, OpenRouterJudge};

    match JevConfig::from_env() {
        Ok(Some(config)) => {
            let judge: Arc<dyn JudgmentModel> = Arc::new(OpenRouterJudge::new(&config));
            server.with_judge(Some(judge))
        }
        Ok(None) => server.with_judge(None),
        Err(error) => {
            tracing::warn!(reason = ?error.reason, "jev disabled");
            server.with_judge(None).with_judge_unavailable(Some(error))
        }
    }
}

#[cfg(not(feature = "jev"))]
fn with_jev(server: FmecaServer) -> FmecaServer {
    server
}

fn run_init(dir: Option<PathBuf>) -> anyhow::Result<()> {
    let root = match dir {
        Some(dir) => dir,
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    for (path, outcome) in init(&root)? {
        let shown = path.strip_prefix(&root).unwrap_or(&path);
        let action = match outcome {
            FileOutcome::Created => "created",
            FileOutcome::Unchanged => "unchanged",
        };
        println!("{action} {}", shown.display());
    }
    Ok(())
}

fn run_check(dir: Option<PathBuf>) -> anyhow::Result<()> {
    let state_dir = dir.unwrap_or_else(resolve_state_dir);
    if !state_dir.is_dir() {
        println!("error: no state dir at {}", state_dir.display());
        std::process::exit(1);
    }

    let mut failed = false;
    for line in check(&state_dir) {
        if line.problems.is_empty() {
            println!("ok {}", line.session_id);
        } else {
            failed = true;
            for problem in line.problems {
                println!("error {}: {}", line.session_id, problem);
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn init_tracing() {
    // Log to stderr so stdout stays the MCP transport channel.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}
