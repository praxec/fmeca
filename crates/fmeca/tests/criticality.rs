//! Criticality-matrix table test: every Severity × Probability
//! combination maps to the fixed High/Medium/Low bucket — golden-tested end to
//! end through the engine projection, not just the bare function.

mod common;

use common::{failure_mode, temp_engine};
use fmeca::Level::{self, High, Low, Medium};
use fmeca::criticality;

/// The authoritative fixed matrix. `[severity][probability]`.
const EXPECTED: [(Level, Level, Level); 9] = [
    (Low, Low, Low),
    (Low, Medium, Low),
    (Low, High, Medium),
    (Medium, Low, Low),
    (Medium, Medium, Medium),
    (Medium, High, High),
    (High, Low, Medium),
    (High, Medium, High),
    (High, High, High),
];

#[test]
fn matrix_function_matches_fixed_table() {
    for (s, p, want) in EXPECTED {
        assert_eq!(
            criticality(s, p),
            want,
            "criticality({s:?},{p:?}) must be {want:?}"
        );
    }
}

#[test]
fn projection_computes_each_matrix_cell() {
    let (engine, _dir) = temp_engine();
    let session = "matrix";
    engine.open_session(session).unwrap();

    for (i, (s, p, want)) in EXPECTED.iter().enumerate() {
        let id = format!("fm{i}");
        let state = engine
            .add_failure_mode(session, failure_mode(session, &id, *s, *p))
            .unwrap();
        let fmv = state
            .failure_modes
            .iter()
            .find(|f| f.failure_mode.id == id)
            .expect("failure mode present");
        assert_eq!(
            fmv.criticality,
            Some(*want),
            "projection criticality for ({s:?},{p:?}) must be {want:?}"
        );
        // With no mitigation, residual == raw criticality.
        assert_eq!(fmv.residual_criticality, Some(*want));
    }
}

#[test]
fn all_nine_cells_are_covered() {
    // Guard against an incomplete table: every (S,P) pair must appear exactly once.
    let mut seen = std::collections::BTreeSet::new();
    for (s, p, _) in EXPECTED {
        assert!(seen.insert((s, p)), "duplicate cell ({s:?},{p:?})");
    }
    assert_eq!(seen.len(), 9, "all nine S×P cells must be present");
}
