#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! `fmeca-mcp` — the stdio MCP server wrapping [`fmeca`].
//!
//! The server is thin: [`FmecaServer`] parses each tool call, invokes one
//! [`Engine`](fmeca::Engine) method, and serializes the result. All
//! structure and state management lives in the kernel (`fmeca`); this crate
//! only translates the MCP wire protocol to/from kernel calls.

pub mod cli;
pub mod config;
#[cfg(feature = "jev")]
pub mod jev;
pub mod project;
pub mod server;
#[cfg(feature = "jev")]
pub mod vet;

pub use config::{
    PROJECT_DIR_NAME, STATE_DIR_ENV, StateDirSource, resolve_state_dir, resolve_state_dir_from,
};
#[cfg(feature = "jev")]
pub use server::TOOL_VET;
pub use server::{
    FmecaServer, TOOL_ANALYZE, TOOL_APPEND, TOOL_NAMES, TOOL_READINESS_ASSESS, TOOL_REPORT_EXPORT,
    TOOL_RISK_NEXT, TOOL_SCORING_CATALOG, TOOL_SESSION_OPEN, TOOL_STATE_GET, tool_definitions,
};
