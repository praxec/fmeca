//! The fixed qualitative criticality matrix.
//!
//! Criticality is a pure function of (severity, probability) over the
//! `Low | Medium | High` scale — a **fixed table**, golden-tested for every one
//! of the nine S×P combinations:
//!
//! ```text
//!            P=Low     P=Medium   P=High
//! S=Low      Low       Low        Medium
//! S=Medium   Low       Medium     High
//! S=High     Medium    High       High
//! ```
//!
//! Rules:
//!  - `High/High`, `High/Med`, `Med/High`           → **High**
//!  - `Med/Med`,  `High/Low`, `Low/High`            → **Medium**
//!  - everything else                               → **Low**

use crate::model::{Criticality, Level};

/// Compute the criticality bucket for an (severity, probability) pair.
///
/// This is the single source of truth for the matrix; the golden table test
/// asserts all nine cells.
pub fn criticality(severity: Level, probability: Level) -> Criticality {
    use Level::{High, Low, Medium};
    match (severity, probability) {
        // → High
        (High, High) | (High, Medium) | (Medium, High) => High,
        // → Medium
        (Medium, Medium) | (High, Low) | (Low, High) => Medium,
        // → Low (Low/Low, Low/Medium, Medium/Low)
        (Low, Low) | (Low, Medium) | (Medium, Low) => Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level::{High, Low, Medium};

    #[test]
    fn matrix_high_bucket() {
        assert_eq!(criticality(High, High), High);
        assert_eq!(criticality(High, Medium), High);
        assert_eq!(criticality(Medium, High), High);
    }

    #[test]
    fn matrix_medium_bucket() {
        assert_eq!(criticality(Medium, Medium), Medium);
        assert_eq!(criticality(High, Low), Medium);
        assert_eq!(criticality(Low, High), Medium);
    }

    #[test]
    fn matrix_low_bucket() {
        assert_eq!(criticality(Low, Low), Low);
        assert_eq!(criticality(Low, Medium), Low);
        assert_eq!(criticality(Medium, Low), Low);
    }

    #[test]
    fn matrix_is_symmetric_in_s_and_p() {
        for s in [Low, Medium, High] {
            for p in [Low, Medium, High] {
                assert_eq!(
                    criticality(s, p),
                    criticality(p, s),
                    "matrix must be symmetric for ({s:?},{p:?})"
                );
            }
        }
    }
}
