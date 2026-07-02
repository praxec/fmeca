//! The kernel engine + session manager.
//!
//! [`Engine`] is the single deterministic entry point: it validates each command
//! payload, appends one event to the [`StateStore`], and recomputes the
//! [`FmecaState`] projection by replay. Every read also projects. There is no
//! in-memory mutable standing — replay is the source of truth, so restart
//! survival is free.
//!
//! ## Per-session write lock
//!
//! Each write command is a TOCTOU read-modify-write: replay → check duplicates /
//! existence → append. Without a per-session lock, two concurrent same-session
//! writes can both pass the check then both append (a dup slips through, or a
//! distinct id is lost via interleaving). Per-session writes are serialized to
//! prevent these races while distinct sessions stay fully concurrent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::error::{FmecaError, Result};
use crate::event::Event;
use crate::matrix::MatrixStrategy;
use crate::model::{FailureMode, Mitigation, Rescore};
use crate::projection::{self, FailureModeView, FmecaState};
use crate::scoring::{self, Axis};
use crate::store::{validate_session_id, StateStore};

/// Per-session lock registry: a `session_id -> lock` map behind a `Mutex`.
///
/// The outer `Mutex` guards only the (brief) map lookup/insert; the per-session
/// `Arc<Mutex<()>>` it hands back is what actually serializes a session's
/// read-modify-write critical section. Holding the per-session lock — not the
/// map lock — across the replay/check/append keeps DIFFERENT sessions fully
/// concurrent while making SAME-session writes mutually exclusive, so the
/// dup/existence checks are race-free.
type LockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;

/// The kernel engine. Cheap to clone (the store and lock map are `Arc`s; clones
/// share the same per-session locks).
#[derive(Clone)]
pub struct Engine {
    store: Arc<dyn StateStore>,
    /// Shared across clones so concurrent operations on the same `session_id` —
    /// even via different `Engine` clones — contend on one lock.
    session_locks: Arc<LockMap>,
}

impl Engine {
    /// Build an engine over the given store.
    pub fn new(store: Arc<dyn StateStore>) -> Self {
        Self {
            store,
            session_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Fetch (or create) the per-session lock. The outer map lock is held only
    /// for the lookup/insert, never across the critical section.
    fn session_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut map = self.session_locks.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(
            map.entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// `session.open`: start or resume a session with the DEFAULT
    /// matrix strategy (3×3). Convenience wrapper over [`Engine::open_session_with`].
    pub fn open_session(&self, session_id: &str) -> Result<FmecaState> {
        self.open_session_with(session_id, MatrixStrategy::default())
    }

    /// `session.open` with an explicit [`MatrixStrategy`]: start or
    /// resume a session. If new, write the `SessionOpened` event RECORDING the
    /// selected strategy so replay is deterministic. If it already exists, the
    /// requested strategy is ignored (a session's strategy is fixed once opened)
    /// and the current state is returned.
    pub fn open_session_with(
        &self,
        session_id: &str,
        matrix_strategy: MatrixStrategy,
    ) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        if !self.store.exists(session_id)? {
            let event = Event::SessionOpened {
                session_id: session_id.to_string(),
                matrix_strategy,
            };
            self.store.append(session_id, &event)?;
        }
        self.state(session_id)
    }

    /// `state.get`: read-only projection of the session.
    pub fn state(&self, session_id: &str) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let events = self.store.replay(session_id)?;
        Ok(projection::project(&events))
    }

    /// `append` variant `add_failure_mode`. Validates the payload,
    /// rejects duplicate ids, appends, and returns the recomputed state.
    pub fn add_failure_mode(&self, session_id: &str, fm: FailureMode) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let events = self.require_session(session_id)?;
        validate_failure_mode(&fm, session_id, session_strategy(&events))?;
        if id_exists(&events, &fm.id) {
            return Err(FmecaError::DuplicateId(fm.id));
        }
        self.store.append(
            session_id,
            &Event::FailureModeAdded {
                failure_mode: Box::new(fm),
            },
        )?;
        self.state(session_id)
    }

    /// `append` variant `add_mitigation`. Validates the payload,
    /// requires the target failure mode to exist, rejects duplicate ids.
    pub fn add_mitigation(&self, session_id: &str, mitigation: Mitigation) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let events = self.require_session(session_id)?;
        validate_mitigation(&mitigation, session_id, session_strategy(&events))?;
        if id_exists(&events, &mitigation.id) {
            return Err(FmecaError::DuplicateId(mitigation.id));
        }
        if !failure_mode_exists(&events, &mitigation.failure_mode_id) {
            return Err(FmecaError::FailureModeNotFound(mitigation.failure_mode_id));
        }
        self.store.append(
            session_id,
            &Event::MitigationAdded {
                mitigation: Box::new(mitigation),
            },
        )?;
        self.state(session_id)
    }

    /// `append` variant `rescore`. Re-scores an existing failure
    /// mode's unmitigated S/P.
    pub fn rescore(&self, session_id: &str, rescore: Rescore) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let events = self.require_session(session_id)?;
        if rescore.session_id != session_id {
            return Err(FmecaError::InvalidRescore(format!(
                "session_id mismatch: payload '{}' vs '{session_id}'",
                rescore.session_id
            )));
        }
        // Note: reject unknown observation ids (under the SESSION'S
        // strategy) before persisting so replay never sees an id outside the
        // active strategy's fixed catalog.
        validate_observations(
            session_strategy(&events),
            &rescore.severity_observations,
            &rescore.probability_observations,
        )?;
        if !failure_mode_exists(&events, &rescore.failure_mode_id) {
            return Err(FmecaError::FailureModeNotFound(rescore.failure_mode_id));
        }
        self.store.append(
            session_id,
            &Event::Rescored {
                rescore: Box::new(rescore),
            },
        )?;
        self.state(session_id)
    }

    /// `risk.next`: the highest-criticality unmitigated failure mode,
    /// or `None`.
    pub fn risk_next(&self, session_id: &str) -> Result<Option<FailureModeView>> {
        let state = self.state(session_id)?;
        Ok(crate::signal::next_risk(&state.failure_modes).cloned())
    }

    /// `readiness.assess`: the readiness report.
    pub fn readiness(&self, session_id: &str) -> Result<crate::readiness::ReadinessReport> {
        Ok(self.state(session_id)?.readiness)
    }

    /// `report.export`: the FMECA report.
    pub fn export(&self, session_id: &str) -> Result<crate::export::FmecaReport> {
        let state = self.state(session_id)?;
        Ok(crate::export::build(&state))
    }

    fn require_session(&self, session_id: &str) -> Result<Vec<Event>> {
        if !self.store.exists(session_id)? {
            return Err(FmecaError::SessionNotFound(session_id.to_string()));
        }
        self.store.replay(session_id)
    }
}

// --- validation helpers ----------------------------------------------------

/// The [`MatrixStrategy`] recorded on a session's `SessionOpened` event. Old
/// logs without the field replayed via serde default to 3×3.
fn session_strategy(events: &[Event]) -> MatrixStrategy {
    events
        .iter()
        .find_map(|e| match e {
            Event::SessionOpened {
                matrix_strategy, ..
            } => Some(*matrix_strategy),
            _ => None,
        })
        .unwrap_or_default()
}

fn validate_failure_mode(
    fm: &FailureMode,
    session_id: &str,
    strategy: MatrixStrategy,
) -> Result<()> {
    if fm.id.is_empty() {
        return Err(FmecaError::InvalidFailureMode(
            "empty failure mode id".into(),
        ));
    }
    if fm.session_id != session_id {
        return Err(FmecaError::InvalidFailureMode(format!(
            "session_id mismatch: payload '{}' vs '{session_id}'",
            fm.session_id
        )));
    }
    if fm.component.id.is_empty() {
        return Err(FmecaError::InvalidFailureMode("empty component id".into()));
    }
    if fm.description.trim().is_empty() {
        return Err(FmecaError::InvalidFailureMode("empty description".into()));
    }
    // Note: observation ids must be in the SESSION'S strategy
    // catalog. The model supplies observations, not scores; an unknown id (or an
    // id from another strategy) is a caller bug.
    validate_observations(
        strategy,
        &fm.severity_observations,
        &fm.probability_observations,
    )?;
    Ok(())
}

/// Reject any observation id outside the active strategy's fixed scoring catalog
///. Empty vectors are fine — they yield an unscored axis (→
/// `missing_score`). Surfaces [`FmecaError::InvalidObservation`] (stable prefix).
fn validate_observations(
    strategy: MatrixStrategy,
    severity_observations: &[String],
    probability_observations: &[String],
) -> Result<()> {
    scoring::derive_ordinal(strategy, Axis::Severity, severity_observations)?;
    scoring::derive_ordinal(strategy, Axis::Probability, probability_observations)?;
    Ok(())
}

fn validate_mitigation(m: &Mitigation, session_id: &str, strategy: MatrixStrategy) -> Result<()> {
    if m.id.is_empty() {
        return Err(FmecaError::InvalidMitigation("empty mitigation id".into()));
    }
    if m.session_id != session_id {
        return Err(FmecaError::InvalidMitigation(format!(
            "session_id mismatch: payload '{}' vs '{session_id}'",
            m.session_id
        )));
    }
    if m.failure_mode_id.is_empty() {
        return Err(FmecaError::InvalidMitigation(
            "empty failure_mode_id".into(),
        ));
    }
    if m.description.trim().is_empty() {
        return Err(FmecaError::InvalidMitigation("empty description".into()));
    }
    // Note: the RESIDUAL is observation-derived too — reject any residual
    // observation id outside the session's strategy catalog (same rule as the
    // unmitigated axes), so an LLM can never estimate the residual number.
    validate_observations(
        strategy,
        &m.residual_severity_observations,
        &m.residual_probability_observations,
    )?;
    Ok(())
}

/// True if `id` collides with any failure-mode or mitigation id already in the
/// log (the cross-kind id namespace is flat, matching `DUPLICATE_ID`).
fn id_exists(events: &[Event], id: &str) -> bool {
    events.iter().any(|e| match e {
        Event::FailureModeAdded { failure_mode } => failure_mode.id == id,
        Event::MitigationAdded { mitigation } => mitigation.id == id,
        Event::SessionOpened { .. } | Event::Rescored { .. } => false,
    })
}

fn failure_mode_exists(events: &[Event], fm_id: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::FailureModeAdded { failure_mode } if failure_mode.id == fm_id))
}
