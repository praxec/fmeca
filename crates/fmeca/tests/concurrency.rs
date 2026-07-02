//! Concurrency safety for the per-session read-modify-write critical section
//! (concurrent sessions are fully isolated; same-session writes must be
//! serialized so the dup/existence checks are race-free).
//!
//! Each engine write command (`open_session`, `add_failure_mode`,
//! `add_mitigation`, `rescore`) is a TOCTOU read-modify-write: it replays the
//! log, checks duplicates / existence, then appends. Without a per-session lock
//! two concurrent operations on the SAME session can both pass the check then
//! both append, so a failure mode is lost or a dup slips past. These tests fan
//! out `std::thread` workers against a shared `Arc<Engine>` and assert the
//! invariants hold deterministically.

mod common;

use std::sync::{Arc, Barrier};
use std::thread;

use common::{failure_mode, temp_engine};
use fmeca::FmecaError;
use fmeca::Level::{High, Low};

#[test]
fn concurrent_distinct_failure_modes_all_persist() {
    const N: usize = 16;
    let (engine, _dir) = temp_engine();
    let engine = Arc::new(engine);
    let session = "concurrent-distinct";
    engine.open_session(session).expect("open");

    let barrier = Arc::new(Barrier::new(N));
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            let fm = failure_mode(session, &format!("fm{i}"), High, Low);
            engine
                .add_failure_mode(session, fm)
                .expect("add_failure_mode must succeed");
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let state = engine.state(session).expect("state");
    assert_eq!(
        state.failure_modes.len(),
        N,
        "all {N} distinct failure modes must be present"
    );
}

#[test]
fn concurrent_same_id_yields_one_success_rest_duplicate() {
    const N: usize = 16;
    let (engine, _dir) = temp_engine();
    let engine = Arc::new(engine);
    let session = "concurrent-same-id";
    engine.open_session(session).expect("open");

    let barrier = Arc::new(Barrier::new(N));
    let mut handles = Vec::with_capacity(N);
    for _ in 0..N {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            let fm = failure_mode(session, "dup", High, Low);
            engine.add_failure_mode(session, fm)
        }));
    }

    let mut successes = 0usize;
    let mut duplicates = 0usize;
    for h in handles {
        match h.join().expect("worker thread panicked") {
            Ok(_) => successes += 1,
            Err(FmecaError::DuplicateId(_)) => duplicates += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert_eq!(successes, 1, "exactly one writer must win the id");
    assert_eq!(duplicates, N - 1, "every other writer must see DuplicateId");

    let state = engine.state(session).expect("state");
    assert_eq!(
        state.failure_modes.len(),
        1,
        "exactly one failure mode must be persisted"
    );
}

#[test]
fn distinct_sessions_isolated() {
    const N: usize = 8;
    let (engine, _dir) = temp_engine();
    let engine = Arc::new(engine);
    let sessions = ["iso-a", "iso-b"];
    for s in sessions {
        engine.open_session(s).expect("open");
    }

    let barrier = Arc::new(Barrier::new(N * sessions.len()));
    let mut handles = Vec::new();
    for s in sessions {
        for i in 0..N {
            let engine = Arc::clone(&engine);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                let fm = failure_mode(s, &format!("{s}-fm{i}"), High, Low);
                engine.add_failure_mode(s, fm).expect("add_failure_mode");
            }));
        }
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }

    for s in sessions {
        let state = engine.state(s).expect("state");
        assert_eq!(
            state.failure_modes.len(),
            N,
            "session {s} must have exactly its own {N} failure modes"
        );
    }
}
