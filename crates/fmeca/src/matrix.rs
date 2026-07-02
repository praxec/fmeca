//! Swappable criticality-matrix **strategy** seam.
//!
//! A [`MatrixStrategy`] is a CLOSED, code-resident set of fixed risk matrices,
//! *selected* per session — never *configured*. Each strategy's cells are LOCKED
//! in code: the caller selects a strategy, it cannot edit cells. This honours the
//! original "fixed/non-configurable matrix" intent, qualified to **per-strategy**
//! cells.
//!
//! Consistent with the no-LLM-numbers rule ([`crate::scoring`]): the model never
//! supplies a level OR a cell value. It supplies OBSERVATIONS; the code derives a
//! level within the *selected strategy's scale* and looks the cell up.
//!
//! ## Two strategies
//!
//! - [`MatrixStrategy::Qualitative3x3`] — the EXISTING 3-level (Low/Medium/High)
//!   behaviour, unchanged. The DEFAULT (back-compat). Its `criticality` is the
//!   historic fixed S×P table in [`crate::criticality`].
//! - [`MatrixStrategy::Nasa8004_5x5`] — a 5-level scale per **NASA GSFC-HDBK-8004**:
//!   consequence/severity `1..=5` and likelihood/probability `1..=5`, with all 25
//!   `(consequence × likelihood)` cells mapped to a [`Criticality`] bucket
//!   {Low|Medium|High} per the standard's risk zones (mapping documented below;
//!   cells locked; golden-tested for all 25 cells).
//!
//! ## Scale representation
//!
//! A strategy's scale is an ordered set of [`StrategyLevel`]s — an ordinal
//! (`1..=N`, ascending = worse) plus a human label. The 3×3 strategy has 3
//! levels; NASA has 5. A strategy's evidence catalog ([`crate::scoring`]) maps an
//! observation id to ONE of its scale's ordinals; `derive_level` MAX-combines
//! ordinals within the strategy. [`MatrixStrategy::criticality`] then collapses
//! the (severity-ordinal, probability-ordinal) pair to the public
//! [`Criticality`] bucket {Low|Medium|High}, so the readiness gate,
//! `response_class`, and the whole downstream pipeline are UNCHANGED — the 5×5's
//! richer input collapses to the L/M/H bucket *at the matrix*.

use serde::{Deserialize, Serialize};

use crate::model::{Criticality, Level};

/// The closed, code-resident set of selectable criticality matrices.
///
/// Selected per session (recorded in `SessionOpened` for deterministic replay);
/// a session's strategy is fixed once opened. The caller SELECTS; it never edits
/// a strategy's cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatrixStrategy {
    /// The historic 3-level qualitative matrix (Low/Medium/High). DEFAULT.
    #[default]
    Qualitative3x3,
    /// The NASA GSFC-HDBK-8004 5×5 risk matrix (consequence×likelihood → zone).
    Nasa8004_5x5,
}

/// One level in a strategy's ordered scale: an ordinal (`1..=N`, higher = worse)
/// and a human label. Surfaced verbatim so the caller knows the strategy's exact
/// vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StrategyLevel {
    /// 1-based rank within the strategy's scale; higher is worse.
    pub ordinal: u8,
    /// Human-readable label for this ordinal (e.g. "high", "catastrophic").
    pub label: String,
}

impl StrategyLevel {
    fn new(ordinal: u8, label: &str) -> Self {
        Self {
            ordinal,
            label: label.to_string(),
        }
    }
}

impl MatrixStrategy {
    /// Stable machine id for this strategy (matches the serde rename).
    pub fn id(self) -> &'static str {
        match self {
            MatrixStrategy::Qualitative3x3 => "qualitative3x3",
            MatrixStrategy::Nasa8004_5x5 => "nasa8004_5x5",
        }
    }

    /// The number of levels on each axis of this strategy's scale.
    pub fn level_count(self) -> u8 {
        match self {
            MatrixStrategy::Qualitative3x3 => 3,
            MatrixStrategy::Nasa8004_5x5 => 5,
        }
    }

    /// The strategy's ordered scale (ascending ordinal = worse). Severity and
    /// probability share the same ordinal scale within a strategy.
    pub fn scale(self) -> Vec<StrategyLevel> {
        match self {
            MatrixStrategy::Qualitative3x3 => vec![
                StrategyLevel::new(1, "low"),
                StrategyLevel::new(2, "medium"),
                StrategyLevel::new(3, "high"),
            ],
            // NASA GSFC-HDBK-8004 5-level scale (1 = least, 5 = worst).
            MatrixStrategy::Nasa8004_5x5 => vec![
                StrategyLevel::new(1, "negligible"),
                StrategyLevel::new(2, "marginal"),
                StrategyLevel::new(3, "moderate"),
                StrategyLevel::new(4, "critical"),
                StrategyLevel::new(5, "catastrophic"),
            ],
        }
    }

    /// True if `ordinal` is a valid rank in this strategy's scale (`1..=N`).
    pub fn is_valid_ordinal(self, ordinal: u8) -> bool {
        ordinal >= 1 && ordinal <= self.level_count()
    }

    /// Collapse a (severity-ordinal, probability-ordinal) pair within this
    /// strategy's scale to the public [`Criticality`] bucket {Low|Medium|High}.
    ///
    /// Ordinals are validated at write time; an out-of-range ordinal here is
    /// clamped to the scale (replay must never panic on persisted data).
    pub fn criticality(self, severity_ordinal: u8, probability_ordinal: u8) -> Criticality {
        let s = severity_ordinal.clamp(1, self.level_count());
        let p = probability_ordinal.clamp(1, self.level_count());
        match self {
            MatrixStrategy::Qualitative3x3 => {
                // Map ordinals back to the historic Level enum and defer to the
                // single source of truth so the 3×3 behaviour is byte-identical.
                crate::criticality::criticality(ordinal_to_level(s), ordinal_to_level(p))
            }
            MatrixStrategy::Nasa8004_5x5 => nasa_5x5_criticality(s, p),
        }
    }
}

/// Map a 3×3 ordinal (`1..=3`) to the historic [`Level`] enum.
fn ordinal_to_level(ordinal: u8) -> Level {
    match ordinal {
        1 => Level::Low,
        2 => Level::Medium,
        _ => Level::High,
    }
}

/// The NASA GSFC-HDBK-8004 5×5 risk-matrix classification.
///
/// Consequence (severity) and likelihood (probability) each run `1..=5` with 5
/// the worst. The standard partitions the 25 cells into three risk zones; we
/// collapse those zones to the public {Low|Medium|High} criticality bucket. The
/// zone boundaries follow the canonical GSFC-HDBK-8004 risk matrix (the same
/// green/yellow/red banding the handbook uses for its 5×5 likelihood/consequence
/// grid):
///
/// ```text
///                          Likelihood (probability) →
///                    L=1     L=2     L=3     L=4     L=5
///   Consequence  C=5  Med     High    High    High    High
///   (severity)   C=4  Med     Med     High    High    High
///       ↓        C=3  Low     Med     Med     High    High
///                C=2  Low     Low     Med     Med     High
///                C=1  Low     Low     Low     Med     Med
/// ```
///
/// Zone rule (symmetric in consequence×likelihood, documented & golden-tested for
/// all 25 cells — the table above is the authoritative locked form and the rule
/// reproduces it exactly):
///  - **High** (red): the high-likelihood high-consequence band — `s + p >= 7`.
///  - **Low** (green): the low-likelihood low-consequence corner — either axis is
///    negligible (`min(s,p) == 1`) while the other is at most moderate
///    (`max(s,p) <= 3`), OR both are marginal (`s == 2 && p == 2`).
///  - **Medium** (yellow): everything in between.
fn nasa_5x5_criticality(s: u8, p: u8) -> Criticality {
    let lo = s.min(p);
    let hi = s.max(p);
    let high = s + p >= 7;
    let low = (lo == 1 && hi <= 3) || (s == 2 && p == 2);
    if high {
        Level::High
    } else if low {
        Level::Low
    } else {
        Level::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level::{High, Low, Medium};

    #[test]
    fn default_strategy_is_qualitative_3x3() {
        assert_eq!(MatrixStrategy::default(), MatrixStrategy::Qualitative3x3);
    }

    #[test]
    fn strategy_ids_are_stable_and_match_serde() {
        assert_eq!(MatrixStrategy::Qualitative3x3.id(), "qualitative3x3");
        assert_eq!(MatrixStrategy::Nasa8004_5x5.id(), "nasa8004_5x5");
        // serde round-trips the same machine ids
        let v = serde_json::to_value(MatrixStrategy::Nasa8004_5x5).unwrap();
        assert_eq!(v, serde_json::json!("nasa8004_5x5"));
        let back: MatrixStrategy = serde_json::from_value(v).unwrap();
        assert_eq!(back, MatrixStrategy::Nasa8004_5x5);
    }

    #[test]
    fn scales_have_the_right_arity_and_ascending_ordinals() {
        let q = MatrixStrategy::Qualitative3x3.scale();
        assert_eq!(q.len(), 3);
        assert_eq!(MatrixStrategy::Qualitative3x3.level_count(), 3);
        let n = MatrixStrategy::Nasa8004_5x5.scale();
        assert_eq!(n.len(), 5);
        assert_eq!(MatrixStrategy::Nasa8004_5x5.level_count(), 5);
        for scale in [q, n] {
            for (i, lvl) in scale.iter().enumerate() {
                assert_eq!(
                    lvl.ordinal as usize,
                    i + 1,
                    "ordinals are 1-based ascending"
                );
            }
        }
    }

    #[test]
    fn qualitative_3x3_collapses_via_historic_table() {
        // Every 3×3 ordinal pair equals the historic criticality() function.
        for s in 1..=3u8 {
            for p in 1..=3u8 {
                let got = MatrixStrategy::Qualitative3x3.criticality(s, p);
                let want =
                    crate::criticality::criticality(ordinal_to_level(s), ordinal_to_level(p));
                assert_eq!(got, want, "3x3 ({s},{p}) must match historic table");
            }
        }
    }

    /// The authoritative locked NASA 5×5 table, `[consequence-1][likelihood-1]`
    /// (consequence/severity rows 1..5, likelihood/probability cols 1..5).
    /// Golden — every one of the 25 cells is asserted.
    const NASA_TABLE: [[Criticality; 5]; 5] = [
        // C=1
        [Low, Low, Low, Medium, Medium],
        // C=2
        [Low, Low, Medium, Medium, High],
        // C=3
        [Low, Medium, Medium, High, High],
        // C=4
        [Medium, Medium, High, High, High],
        // C=5
        [Medium, High, High, High, High],
    ];

    #[test]
    fn nasa_5x5_matches_golden_table_for_all_25_cells() {
        let strat = MatrixStrategy::Nasa8004_5x5;
        let mut seen = 0usize;
        for c in 1..=5u8 {
            for l in 1..=5u8 {
                let want = NASA_TABLE[(c - 1) as usize][(l - 1) as usize];
                let got = strat.criticality(c, l);
                assert_eq!(got, want, "NASA 5x5 cell (C={c}, L={l}) must be {want:?}");
                seen += 1;
            }
        }
        assert_eq!(seen, 25, "all 25 NASA cells must be covered");
    }

    #[test]
    fn nasa_5x5_corners_are_intuitive() {
        let n = MatrixStrategy::Nasa8004_5x5;
        assert_eq!(n.criticality(1, 1), Low, "least/least is Low");
        assert_eq!(n.criticality(5, 5), High, "worst/worst is High");
        assert_eq!(n.criticality(3, 3), Medium, "centre is Medium");
    }

    #[test]
    fn out_of_range_ordinal_is_clamped_not_panicked() {
        // Replay safety: never panic on a persisted out-of-range ordinal. Each
        // axis clamps into [1, N].
        assert_eq!(MatrixStrategy::Nasa8004_5x5.criticality(0, 0), Low); // → (1,1)
        assert_eq!(MatrixStrategy::Nasa8004_5x5.criticality(99, 99), High); // → (5,5)
        assert_eq!(MatrixStrategy::Qualitative3x3.criticality(0, 99), Medium); // → (1,3)
    }

    #[test]
    fn ordinal_validity_tracks_scale_arity() {
        assert!(MatrixStrategy::Qualitative3x3.is_valid_ordinal(3));
        assert!(!MatrixStrategy::Qualitative3x3.is_valid_ordinal(4));
        assert!(MatrixStrategy::Nasa8004_5x5.is_valid_ordinal(5));
        assert!(!MatrixStrategy::Nasa8004_5x5.is_valid_ordinal(6));
        assert!(!MatrixStrategy::Nasa8004_5x5.is_valid_ordinal(0));
    }
}
