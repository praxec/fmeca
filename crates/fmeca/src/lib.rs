// Restriction-category lint on production code only. `cargo test` compiles with
// `cfg(test)`, which silences this everywhere — production code propagates
// errors; tests may `unwrap`.
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! `fmeca` — the deterministic, offline kernel for structured **FMECA**
//! (Failure Modes, Effects & Criticality Analysis).
//!
//! The caller (an LLM) does the fuzzy work — naming failure modes, causes,
//! effects, and proposing mitigations. This kernel owns **structure**: a typed
//! failure-mode / mitigation ledger, the fixed qualitative criticality matrix
//! (S×P → High/Medium/Low), the prevent→detect→fail-fast mitigation-order
//! discipline, computed residual risk, gap detection with notify/clarify/
//! remediate signals, a readiness gate, and report export.
//!
//! Criticality, residual, and standing are **computed** — a pure fold over an
//! append-only event log, never stored. No LLM, no network. The kernel is a pure
//! function of `(events in) → (criticality + residual + gaps + readiness out)`.
//!
//! # Layout
//!
//! - [`model`]       — the typed data model (failure mode, mitigation, entity,
//!   levels, criticality, domain, issue).
//! - [`scoring`]     — the fixed code-resident evidence→score catalog + the
//!   observation→Level map (the model never picks a score).
//! - [`response`]    — the deterministic `response_class` magnitude derivation.
//! - [`criticality`] — the fixed qualitative S×P matrix (the 3×3 strategy's cells).
//! - [`matrix`]      — the swappable [`MatrixStrategy`](matrix::MatrixStrategy)
//!   seam: a closed set of fixed matrices selected per session (3×3
//!   default + NASA GSFC-HDBK-8004 5×5), each with locked cells.
//! - [`event`]       — the append-only event log line type.
//! - [`store`]       — the [`StateStore`](store::StateStore) trait + filesystem
//!   (JSONL) impl.
//! - [`projection`]  — the [`FmecaState`](projection::FmecaState) fold + computed
//!   criticality/residual/standing.
//! - [`detect`]      — issue detectors (one per `IssueType`).
//! - [`signal`]      — the three first-class signals + `risk.next`.
//! - [`readiness`]   — the readiness gate.
//! - [`export`]      — `report.export`.
//! - [`engine`]      — the [`Engine`](engine::Engine) tying it together, with the
//!   per-session write lock.

pub mod analyze;
pub mod criticality;
mod detect;
pub mod engine;
pub mod error;
pub mod event;
pub mod export;
pub mod matrix;
pub mod model;
pub mod projection;
pub mod readiness;
pub mod response;
pub mod scoring;
pub mod signal;
pub mod store;

pub use analyze::{AnalyzeFailureMode, AnalyzeInput, AnalyzeMitigation, AnalyzeReport, analyze};
pub use criticality::criticality;
pub use engine::Engine;
pub use error::{FmecaError, Result};
pub use event::Event;
pub use export::{ExportedMitigation, FmecaReport, FmecaRow};
pub use matrix::{MatrixStrategy, StrategyLevel};
pub use model::{
    Criticality, Domain, EntityRef, EvidenceRef, FailureMode, FailureModeStanding, Issue,
    IssueType, Level, Mitigation, MitigationKind, Rescore,
};
pub use projection::{FailureModeView, FmecaState, Registry};
pub use readiness::{CriticalityBuckets, ReadinessReport};
pub use response::{ResponseClass, Scope, response_class};
pub use scoring::{Axis, ScoreCriterion, catalog, catalog_for, derive_level, derive_ordinal};
pub use signal::{Signal, SignalKind, next_risk};
pub use store::{FilesystemStore, StateStore};
