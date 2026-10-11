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
use crate::model::{
    Disposition, DispositionChange, FailureMode, Mitigation, MitigationStatus,
    MitigationStatusChange, ParentRef, Rescore, ResolutionEvidence, SessionMeta,
};
use crate::projection::{self, FailureModeView, FmecaState};
use crate::scoring::{self, Axis};
use crate::store::{StateStore, validate_session_id};

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
        self.open_session_with_meta(session_id, matrix_strategy, SessionMeta::default())
    }

    /// `session.open` with a strategy and analysis metadata (spec §6.2). A
    /// `meta.parent` must name an existing session, else `SESSION_NOT_FOUND`.
    /// Like the strategy, the metadata of an existing session is never changed:
    /// reopening ignores both and returns the current state.
    pub fn open_session_with_meta(
        &self,
        session_id: &str,
        matrix_strategy: MatrixStrategy,
        meta: SessionMeta,
    ) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        if !self.store.exists(session_id)? {
            if let Some(parent) = &meta.parent
                && (validate_session_id(parent).is_err() || !self.store.exists(parent)?)
            {
                return Err(FmecaError::SessionNotFound(parent.clone()));
            }
            let event = Event::SessionOpened {
                session_id: session_id.to_string(),
                matrix_strategy,
                meta,
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
        if let Some(parent) = &fm.parent {
            self.require_parent_failure_mode(session_id, &events, parent)?;
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

    /// `append` variant `set_mitigation_status` (spec §6.3). The mitigation must
    /// exist; `Verified` needs at least one evidence item that is not a note.
    /// Any transition is allowed; every change is kept in the history.
    pub fn set_mitigation_status(
        &self,
        session_id: &str,
        change: MitigationStatusChange,
    ) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let events = self.require_session(session_id)?;
        if change.session_id != session_id {
            return Err(FmecaError::InvalidMitigation(format!(
                "session_id mismatch: payload '{}' vs '{session_id}'",
                change.session_id
            )));
        }
        if !mitigation_exists(&events, &change.mitigation_id) {
            return Err(FmecaError::InvalidMitigation(format!(
                "mitigation not found: {}",
                change.mitigation_id
            )));
        }
        if change.status == MitigationStatus::Verified && !has_substantive(&change.evidence) {
            return Err(FmecaError::InvalidMitigation(
                "verified requires evidence other than a note".into(),
            ));
        }
        self.store.append(
            session_id,
            &Event::MitigationStatusChanged {
                change: Box::new(change),
            },
        )?;
        self.state(session_id)
    }

    /// `append` variant `set_failure_mode_disposition` (spec §6.6.2). The
    /// failure mode must exist and `rationale` must be non-empty; `Accepted`
    /// needs an approver, `Excluded` needs evidence that is not a note.
    pub fn set_failure_mode_disposition(
        &self,
        session_id: &str,
        change: DispositionChange,
    ) -> Result<FmecaState> {
        validate_session_id(session_id)?;
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        let events = self.require_session(session_id)?;
        if change.session_id != session_id {
            return Err(FmecaError::InvalidFailureMode(format!(
                "session_id mismatch: payload '{}' vs '{session_id}'",
                change.session_id
            )));
        }
        if !failure_mode_exists(&events, &change.failure_mode_id) {
            return Err(FmecaError::FailureModeNotFound(change.failure_mode_id));
        }
        if change.rationale.trim().is_empty() {
            return Err(FmecaError::InvalidFailureMode(
                "disposition requires a rationale".into(),
            ));
        }
        match change.disposition {
            Disposition::Accepted
                if change
                    .approver
                    .as_deref()
                    .is_none_or(|a| a.trim().is_empty()) =>
            {
                return Err(FmecaError::InvalidFailureMode(
                    "accepted requires an approver".into(),
                ));
            }
            Disposition::Excluded if !has_substantive(&change.evidence) => {
                return Err(FmecaError::InvalidFailureMode(
                    "excluded requires evidence other than a note".into(),
                ));
            }
            _ => {}
        }
        self.store.append(
            session_id,
            &Event::FailureModeDispositioned {
                change: Box::new(change),
            },
        )?;
        self.state(session_id)
    }

    /// Session ids in the store, sorted (see [`StateStore::list_sessions`]).
    pub fn list_sessions(&self) -> Result<Vec<String>> {
        self.store.list_sessions()
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

    /// A failure mode's `parent` must name an existing session that contains
    /// that failure-mode id. Checked on append only, never on replay.
    fn require_parent_failure_mode(
        &self,
        session_id: &str,
        events: &[Event],
        parent: &ParentRef,
    ) -> Result<()> {
        let found = if parent.session_id == session_id {
            failure_mode_exists(events, &parent.failure_mode_id)
        } else if validate_session_id(&parent.session_id).is_ok()
            && self.store.exists(&parent.session_id)?
        {
            let parent_events = self.store.replay(&parent.session_id)?;
            failure_mode_exists(&parent_events, &parent.failure_mode_id)
        } else {
            false
        };
        if found {
            Ok(())
        } else {
            Err(FmecaError::InvalidFailureMode(format!(
                "parent failure mode not found: {}/{}",
                parent.session_id, parent.failure_mode_id
            )))
        }
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
        Event::SessionOpened { .. }
        | Event::Rescored { .. }
        | Event::MitigationStatusChanged { .. }
        | Event::FailureModeDispositioned { .. } => false,
    })
}

fn mitigation_exists(events: &[Event], mitigation_id: &str) -> bool {
    events.iter().any(
        |e| matches!(e, Event::MitigationAdded { mitigation } if mitigation.id == mitigation_id),
    )
}

/// True when at least one evidence item is something other than a note.
fn has_substantive(evidence: &[ResolutionEvidence]) -> bool {
    evidence.iter().any(ResolutionEvidence::is_substantive)
}

fn failure_mode_exists(events: &[Event], fm_id: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::FailureModeAdded { failure_mode } if failure_mode.id == fm_id))
}
