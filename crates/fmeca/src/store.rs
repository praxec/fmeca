//! The event-log persistence seam.
//!
//! [`StateStore`] is the only I/O boundary the kernel knows about: append one
//! event, replay a session's events, check existence. The filesystem impl writes
//! one JSON event per line to `<state_dir>/<session_id>.jsonl`.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::error::{FmecaError, Result};
use crate::event::Event;

/// Persistence for the per-session append-only event log.
///
/// Implementations must preserve append order and survive process restarts.
pub trait StateStore: Send + Sync {
    /// Append one event to the session's log, creating it if absent.
    fn append(&self, session_id: &str, event: &Event) -> Result<()>;

    /// Replay all events for a session, in append order. Returns
    /// [`FmecaError::SessionNotFound`] if the session has never been opened.
    fn replay(&self, session_id: &str) -> Result<Vec<Event>>;

    /// True if the session has an event log.
    fn exists(&self, session_id: &str) -> Result<bool>;
}

/// Reject session ids that are empty or would escape the state dir / collide
/// with the `.jsonl` layout. The kernel keys every file by `session_id`, so this
/// is the durability boundary's safety check (path-safe ids).
pub fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() {
        return Err(FmecaError::BadSessionId("empty session id".into()));
    }
    // Disallow path separators, control chars, and traversal. Dots are allowed
    // inside an id, but a leading `.` or any `..` segment is rejected so an id
    // can never escape the state dir or hide a file.
    let has_illegal_char = session_id
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_control());
    if has_illegal_char || session_id.contains("..") || session_id.starts_with('.') {
        return Err(FmecaError::BadSessionId(format!(
            "illegal characters in session id '{session_id}'"
        )));
    }
    Ok(())
}

/// Filesystem-backed [`StateStore`]. One `.jsonl` file per session
/// under `state_dir`.
#[derive(Debug, Clone)]
pub struct FilesystemStore {
    state_dir: PathBuf,
}

impl FilesystemStore {
    /// Create a store rooted at `state_dir`, creating the directory if needed.
    pub fn new(state_dir: impl Into<PathBuf>) -> Result<Self> {
        let state_dir = state_dir.into();
        fs::create_dir_all(&state_dir)
            .map_err(|e| FmecaError::StoreError(format!("create_dir_all {state_dir:?}: {e}")))?;
        Ok(Self { state_dir })
    }

    /// The on-disk path for a session's log.
    fn path_for(&self, session_id: &str) -> Result<PathBuf> {
        validate_session_id(session_id)?;
        Ok(self.state_dir.join(format!("{session_id}.jsonl")))
    }

    /// The configured state directory.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }
}

impl StateStore for FilesystemStore {
    fn append(&self, session_id: &str, event: &Event) -> Result<()> {
        let path = self.path_for(session_id)?;
        let line = serde_json::to_string(event)
            .map_err(|e| FmecaError::StoreError(format!("serialize event: {e}")))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| FmecaError::StoreError(format!("open {path:?}: {e}")))?;
        writeln!(file, "{line}")
            .map_err(|e| FmecaError::StoreError(format!("write {path:?}: {e}")))?;
        file.flush()
            .map_err(|e| FmecaError::StoreError(format!("flush {path:?}: {e}")))?;
        Ok(())
    }

    fn replay(&self, session_id: &str) -> Result<Vec<Event>> {
        let path = self.path_for(session_id)?;
        if !path.exists() {
            return Err(FmecaError::SessionNotFound(session_id.to_string()));
        }
        let file =
            File::open(&path).map_err(|e| FmecaError::StoreError(format!("open {path:?}: {e}")))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| FmecaError::StoreError(format!("read line: {e}")))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = serde_json::from_str(&line).map_err(|e| {
                FmecaError::StoreError(format!("parse {path:?} line {}: {e}", idx + 1))
            })?;
            events.push(event);
        }
        Ok(events)
    }

    fn exists(&self, session_id: &str) -> Result<bool> {
        Ok(self.path_for(session_id)?.exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(session_id: &str) -> Event {
        Event::SessionOpened {
            session_id: session_id.to_string(),
            matrix_strategy: crate::matrix::MatrixStrategy::default(),
        }
    }

    #[test]
    fn append_then_replay_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemStore::new(dir.path()).unwrap();
        let e = opened("s1");
        store.append("s1", &e).unwrap();
        let back = store.replay("s1").unwrap();
        assert_eq!(back, vec![e]);
    }

    #[test]
    fn replay_unknown_session_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilesystemStore::new(dir.path()).unwrap();
        let err = store.replay("nope").unwrap_err();
        assert!(matches!(err, FmecaError::SessionNotFound(_)));
    }

    #[test]
    fn bad_session_ids_rejected() {
        for bad in ["", "../escape", "a/b", ".hidden", "with\0nul"] {
            assert!(validate_session_id(bad).is_err(), "should reject {bad:?}");
        }
        for ok in ["s1", "session-2026", "a.b.c", "UUID_1234"] {
            assert!(validate_session_id(ok).is_ok(), "should accept {ok:?}");
        }
    }
}
