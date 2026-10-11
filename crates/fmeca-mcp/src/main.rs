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

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::cli::{Command, USAGE, parse};
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
    }
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
