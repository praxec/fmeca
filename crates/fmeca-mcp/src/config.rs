//! Minimal server configuration: the state directory.
//!
//! Resolution order for `state_dir`:
//!  1. `FMECA_STATE_DIR` env var (when set and non-empty),
//!  2. the nearest ancestor of the working directory that contains a
//!     `.fmeca/` **directory**,
//!  3. `$XDG_DATA_HOME/fmeca-mcp` or `$HOME/.local/share/fmeca-mcp`,
//!  4. `./fmeca-state` as a last resort.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Environment variable overriding the state directory.
pub const STATE_DIR_ENV: &str = "FMECA_STATE_DIR";

/// Name of the in-project state directory created by `fmeca-mcp init`.
pub const PROJECT_DIR_NAME: &str = ".fmeca";

/// Which rule resolved the state directory. Useful for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateDirSource {
    /// `FMECA_STATE_DIR` was set.
    Env,
    /// A `.fmeca/` directory was found at or above the working directory.
    Project,
    /// The user data directory was used.
    User,
    /// Nothing else applied; the relative `fmeca-state` fallback was used.
    Fallback,
}

/// Pure resolver: environment lookup + starting directory → resolved dir and
/// the rule that applied. Testable without touching the process environment.
pub fn resolve_state_dir_from(
    get_env: impl Fn(&str) -> Option<OsString>,
    cwd: &Path,
) -> (PathBuf, StateDirSource) {
    if let Some(dir) = get_env(STATE_DIR_ENV)
        && !dir.is_empty()
    {
        return (PathBuf::from(dir), StateDirSource::Env);
    }

    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(PROJECT_DIR_NAME);
        if candidate.is_dir() {
            return (candidate, StateDirSource::Project);
        }
    }

    if let Some(xdg) = get_env("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        return (PathBuf::from(xdg).join("fmeca-mcp"), StateDirSource::User);
    }
    if let Some(home) = get_env("HOME")
        && !home.is_empty()
    {
        return (
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("fmeca-mcp"),
            StateDirSource::User,
        );
    }

    (PathBuf::from("fmeca-state"), StateDirSource::Fallback)
}

/// Resolve the on-disk state directory from the process environment and the
/// current working directory.
pub fn resolve_state_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_state_dir_from(|key| std::env::var_os(key), &cwd).0
}
