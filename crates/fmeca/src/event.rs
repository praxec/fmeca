//! The append-only event log.
//!
//! One [`Event`] per line of `<state_dir>/<session_id>.jsonl`. [`FmecaState`]
//! is rebuilt purely by replaying these events in order — standing, criticality,
//! and residual are never stored; provenance is free, appends are crash-resilient,
//! restart-survival is just replay.
//!
//! [`FmecaState`]: crate::projection::FmecaState

use serde::{Deserialize, Serialize};

use crate::matrix::MatrixStrategy;
use crate::model::{FailureMode, Mitigation, Rescore};

/// A single line in a session's append-only log. The `type` tag keeps the JSONL
/// self-describing and forward-compatible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Session opened. The first line of every log. Records the selected
    /// [`MatrixStrategy`] so replay is deterministic and the choice is
    /// RECORDED. Old logs without the field replay as the default 3×3 strategy.
    SessionOpened {
        session_id: String,
        #[serde(default)]
        matrix_strategy: MatrixStrategy,
    },
    /// A failure mode was added (`append` variant `add_failure_mode`). Boxed to
    /// keep this variant from dominating the enum's size (clippy
    /// `large_enum_variant`).
    FailureModeAdded { failure_mode: Box<FailureMode> },
    /// A mitigation was added (`append` variant `add_mitigation`).
    MitigationAdded { mitigation: Box<Mitigation> },
    /// A failure mode's unmitigated S/P was re-scored (`append` variant
    /// `rescore`).
    Rescored { rescore: Box<Rescore> },
}

impl Event {
    /// The session this event belongs to.
    pub fn session_id(&self) -> &str {
        match self {
            Event::SessionOpened { session_id, .. } => session_id,
            Event::FailureModeAdded { failure_mode } => &failure_mode.session_id,
            Event::MitigationAdded { mitigation } => &mitigation.session_id,
            Event::Rescored { rescore } => &rescore.session_id,
        }
    }
}
