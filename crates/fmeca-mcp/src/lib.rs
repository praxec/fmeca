#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! `fmeca-mcp` — the stdio MCP server wrapping [`fmeca`].
//!
//! The server is thin: [`FmecaServer`] parses each tool call, invokes one
//! [`Engine`](fmeca::Engine) method, and serializes the result. All
//! structure and state management lives in the kernel (`fmeca`); this crate
//! only translates the MCP wire protocol to/from kernel calls.

pub mod config;
pub mod server;

pub use config::{STATE_DIR_ENV, resolve_state_dir};
pub use server::{
    FmecaServer, TOOL_ANALYZE, TOOL_APPEND, TOOL_NAMES, TOOL_READINESS_ASSESS, TOOL_REPORT_EXPORT,
    TOOL_RISK_NEXT, TOOL_SCORING_CATALOG, TOOL_SESSION_OPEN, TOOL_STATE_GET, tool_definitions,
};
