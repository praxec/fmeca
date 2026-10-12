//! Structural validation of a parsed [`Policy`] (spec §3).

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Conditions, Matrix, Policy, SCHEMA_VERSION};
use crate::{PolicyError, invalid};

/// The S×P matrix layouts and their sizes: key (= `derived_from`) → N for N×N.
const SXP_MATRICES: [(&str, usize); 2] = [("qualitative3x3", 3), ("nasa8004_5x5", 5)];

/// The key of the optional confidence matrix.
const CONFIDENCE_KEY: &str = "confidence";
/// The confidence matrix's `derived_from`.
const CONFIDENCE_DERIVED_FROM: &str = "standard-bands";
/// Rows of the confidence matrix: one per band of the built-in `standard`
/// preset (spec §3, §6), by rank ascending.
const STANDARD_BAND_COUNT: usize = 3;
/// Columns of the confidence matrix: `C4`…`C0`.
const CONFIDENCE_COLUMNS: usize = 5;

/// Check `policy` against the spec §3 rules. The first violation found is
/// returned as [`PolicyError::Invalid`] naming the offending field.
pub fn validate(policy: &Policy) -> Result<(), PolicyError> {
    if policy.schema != SCHEMA_VERSION {
        return Err(invalid(
            "schema",
            format!(
                "unsupported schema {}; expected {SCHEMA_VERSION}",
                policy.schema
            ),
        ));
    }
    let ranks = band_ranks(policy)?;
    validate_matrices(&policy.matrices, &ranks)?;
    if !ranks.contains_key(policy.signoff.max_residual_band.as_str()) {
        return Err(unknown_band(
            "signoff.max_residual_band",
            &policy.signoff.max_residual_band,
        ));
    }
    let mut action_ids = BTreeSet::new();
    for (i, action) in policy.actions.iter().enumerate() {
        if !action_ids.insert(action.id.as_str()) {
            return Err(invalid(
                format!("actions[{i}].id"),
                format!("duplicate action id `{}`", action.id),
            ));
        }
        validate_conditions(
            &format!("actions[{i}].applies_when"),
            &action.applies_when,
            &ranks,
        )?;
        if let Some(required) = &action.required_when {
            validate_conditions(&format!("actions[{i}].required_when"), required, &ranks)?;
        }
    }
    Ok(())
}

/// Parse a duration of the form `<n>d` into days: `"90d"` → `90`.
pub fn parse_days(s: &str) -> Result<u32, PolicyError> {
    let not_a_duration = || {
        invalid(
            "duration",
            format!("`{s}` is not a duration of the form <n>d"),
        )
    };
    let digits = s.strip_suffix('d').ok_or_else(not_a_duration)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(not_a_duration());
    }
    digits.parse().map_err(|_| not_a_duration())
}

/// Band id → rank, after checking ids and ranks are unique and durations
/// well formed.
fn band_ranks(policy: &Policy) -> Result<BTreeMap<&str, u8>, PolicyError> {
    if policy.bands.is_empty() {
        return Err(invalid("bands", "a policy needs at least one band"));
    }
    let mut ranks = BTreeMap::new();
    let mut seen_ranks = BTreeSet::new();
    for (i, band) in policy.bands.iter().enumerate() {
        if ranks.insert(band.id.as_str(), band.rank).is_some() {
            return Err(invalid(
                format!("bands[{i}].id"),
                format!("duplicate band id `{}`", band.id),
            ));
        }
        if !seen_ranks.insert(band.rank) {
            return Err(invalid(
                format!("bands[{i}].rank"),
                format!("duplicate band rank {}", band.rank),
            ));
        }
        if let Some(interval) = &band.review_interval {
            parse_days(interval).map_err(|e| match e {
                PolicyError::Invalid { reason, .. } => {
                    invalid(format!("bands[{i}].review_interval"), reason)
                }
                other => other,
            })?;
        }
    }
    Ok(ranks)
}

fn validate_matrices(
    matrices: &BTreeMap<String, Matrix>,
    ranks: &BTreeMap<&str, u8>,
) -> Result<(), PolicyError> {
    if !SXP_MATRICES
        .iter()
        .any(|(key, _)| matrices.contains_key(*key))
    {
        return Err(invalid(
            "matrices",
            "a policy needs at least one S×P matrix (`qualitative3x3` or `nasa8004_5x5`)",
        ));
    }
    for (key, matrix) in matrices {
        let field = format!("matrices.{key}");
        let (expected_from, rows, cols) =
            if let Some((_, n)) = SXP_MATRICES.iter().find(|(k, _)| k == key) {
                (key.as_str(), *n, *n)
            } else if key == CONFIDENCE_KEY {
                (
                    CONFIDENCE_DERIVED_FROM,
                    STANDARD_BAND_COUNT,
                    CONFIDENCE_COLUMNS,
                )
            } else {
                return Err(invalid(
                    field,
                    "unknown matrix; expected `qualitative3x3`, `nasa8004_5x5` or `confidence`",
                ));
            };
        if matrix.derived_from != expected_from {
            return Err(invalid(
                format!("{field}.derived_from"),
                format!(
                    "`{}` does not match this matrix; expected `{expected_from}`",
                    matrix.derived_from
                ),
            ));
        }
        if matrix.cells.len() != rows || matrix.cells.iter().any(|row| row.len() != cols) {
            return Err(invalid(
                format!("{field}.cells"),
                format!("expected {rows}×{cols} cells"),
            ));
        }
        let grid = cell_ranks(&field, matrix, ranks)?;
        check_monotone(&field, &grid)?;
    }
    Ok(())
}

/// The rank of every cell, checking each names a band.
fn cell_ranks(
    field: &str,
    matrix: &Matrix,
    ranks: &BTreeMap<&str, u8>,
) -> Result<Vec<Vec<u8>>, PolicyError> {
    matrix
        .cells
        .iter()
        .enumerate()
        .map(|(r, row)| {
            row.iter()
                .enumerate()
                .map(|(c, id)| {
                    ranks
                        .get(id.as_str())
                        .copied()
                        .ok_or_else(|| unknown_band(&format!("{field}.cells[{r}][{c}]"), id))
                })
                .collect()
        })
        .collect()
}

/// A cell's rank never decreases along a row (the column axis increases) or
/// down a column (the row axis increases). For S×P matrices that is severity
/// and probability increasing; for `confidence`, band rank increasing and
/// confidence decreasing.
fn check_monotone(field: &str, grid: &[Vec<u8>]) -> Result<(), PolicyError> {
    for (r, row) in grid.iter().enumerate() {
        for (c, &rank) in row.iter().enumerate() {
            let left = c.checked_sub(1).map(|pc| row[pc]);
            let above = r.checked_sub(1).map(|pr| grid[pr][c]);
            if left.is_some_and(|l| rank < l) || above.is_some_and(|a| rank < a) {
                return Err(invalid(
                    format!("{field}.cells[{r}][{c}]"),
                    "matrix is not monotone: a band's rank decreases as an axis increases",
                ));
            }
        }
    }
    Ok(())
}

fn validate_conditions(
    field: &str,
    conditions: &Conditions,
    ranks: &BTreeMap<&str, u8>,
) -> Result<(), PolicyError> {
    if let Some(band) = &conditions.band_at_least
        && !ranks.contains_key(band.as_str())
    {
        return Err(unknown_band(&format!("{field}.band_at_least"), band));
    }
    for (j, band) in conditions.band_in.iter().flatten().enumerate() {
        if !ranks.contains_key(band.as_str()) {
            return Err(unknown_band(&format!("{field}.band_in[{j}]"), band));
        }
    }
    Ok(())
}

fn unknown_band(field: &str, id: &str) -> PolicyError {
    invalid(field, format!("unknown band `{id}`"))
}
