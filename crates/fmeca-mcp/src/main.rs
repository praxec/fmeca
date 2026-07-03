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

use std::sync::Arc;

use fmeca::{Engine, FilesystemStore};
use fmeca_mcp::{FmecaServer, resolve_state_dir};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let state_dir = resolve_state_dir();
    tracing::info!(?state_dir, "starting fmeca-mcp stdio server");
    let store = FilesystemStore::new(&state_dir)?;
    let engine = Engine::new(Arc::new(store));
    let server = FmecaServer::new(engine);
    server.serve_stdio().await?;
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
