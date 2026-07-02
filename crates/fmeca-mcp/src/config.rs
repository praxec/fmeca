//! Minimal server configuration: the state directory.
//!
//! Resolution order for `state_dir`:
//!  1. `FMECA_STATE_DIR` env var,
//!  2. `$XDG_DATA_HOME/fmeca-mcp` or `$HOME/.local/share/fmeca-mcp`,
//!  3. `./fmeca-state` as a last resort.

use std::path::PathBuf;

/// Environment variable overriding the state directory.
pub const STATE_DIR_ENV: &str = "FMECA_STATE_DIR";

/// Resolve the on-disk state directory from the environment.
pub fn resolve_state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(STATE_DIR_ENV) {
        return PathBuf::from(dir);
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("fmeca-mcp");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("fmeca-mcp");
    }
    PathBuf::from("fmeca-state")
}
