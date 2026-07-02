//! Kernel error type with stable, machine-parseable prefixes.
//!
//! The `Display` output of each variant begins with a stable uppercase code
//! (`SESSION_NOT_FOUND`, `INVALID_FAILURE_MODE`, `BAD_SESSION_ID`, …) so the MCP
//! server can surface them verbatim and clients can pattern-match on the prefix.

use thiserror::Error;

/// All errors the kernel can return. Each `Display` begins with a stable code.
#[derive(Debug, Error)]
pub enum FmecaError {
    /// The referenced session has no event log.
    #[error("SESSION_NOT_FOUND: no session '{0}'")]
    SessionNotFound(String),

    /// The session id is empty or contains path-unsafe characters.
    #[error("BAD_SESSION_ID: {0}")]
    BadSessionId(String),

    /// A failure-mode payload is structurally invalid (empty id, mismatched
    /// session, etc.).
    #[error("INVALID_FAILURE_MODE: {0}")]
    InvalidFailureMode(String),

    /// A mitigation payload is invalid (empty id, mismatched session, etc.).
    #[error("INVALID_MITIGATION: {0}")]
    InvalidMitigation(String),

    /// A rescore payload is invalid.
    #[error("INVALID_RESCORE: {0}")]
    InvalidRescore(String),

    /// An observation id is not in the fixed scoring catalog.
    #[error("INVALID_OBSERVATION: {0}")]
    InvalidObservation(String),

    /// A referenced failure mode does not exist in the session.
    #[error("FM_NOT_FOUND: no failure mode '{0}'")]
    FailureModeNotFound(String),

    /// A duplicate failure-mode / mitigation id was submitted.
    #[error("DUPLICATE_ID: {0}")]
    DuplicateId(String),

    /// Underlying store I/O failure.
    #[error("STORE_ERROR: {0}")]
    StoreError(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, FmecaError>;
