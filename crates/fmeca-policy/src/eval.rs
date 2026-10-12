//! The deterministic evaluator (spec §4): a [`Subject`], a validated policy
//! and an `as_of` date in, tagged [`Suggestions`] and a policy-relative
//! sign-off verdict out.
//!
//! [`evaluate`] is a pure function: no clock, no randomness. Failure modes
//! are processed in id order and every map is a [`BTreeMap`], so the same
//! inputs always serialise to byte-identical JSON.
//!
//! # Rules (spec §4)
//!
//! 1. **Inherent band**: the policy matrix cell for the unmitigated
//!    `(severity, probability)` ordinals (`cells[s - 1][p - 1]`). Unscored
//!    failure modes have no band, are C0, and fail sign-off (`<fm>:
//!    unscored`) whatever `include_kernel_gate` says.
//! 2. **Residual band**: the cell for [`SubjectFm::residual`] (the kernel's
//!    best residual over the active mitigations). With no residual, the
//!    kernel's rule applies and the residual is the inherent band.
//! 3. **Confidence**: [`residual_confidence`].
//! 4. **Effective band**: the residual band, unless the confidence is below
//!    the required confidence — the highest `min_confidence` over the
//!    residual band and every band the claimed reduction leaves (ranked
//!    above the residual band, at or below the inherent band); then the
//!    inherent band, reported as a [`Fallback`]. Claiming to leave a band
//!    needs that band's evidence, so the threshold is monotone in the
//!    claimed reduction. A fallback only ever raises the band: it applies
//!    when the inherent band ranks above the residual band.
//! 5. **Disposition**: accepted (with review status), or a suggested
//!    disposition for an open failure mode; `rationale_needed`.
//! 6. **Actions**: the catalog actions whose `applies_when` holds, sorted by
//!    precedence then id, marked required and covered.
//!
//! **Sign-off**: an effective band above `max_residual_band` fails unless the
//! failure mode is accepted, `accepted_counts_as_pass` holds, the review is
//! not overdue (when `accepted_requires_review_current`), and the effective
//! band's `dispositions` include `accept`. With `include_kernel_gate`, the
//! kernel's hygiene blockers ([`KernelGate`](crate::KernelGate): CLARIFY,
//! weak mitigation order, EMPTY — never its residual-zone blockers) are added
//! as `kernel: <blocker>`.
//!
//! # Confidence-view policies
//!
//! A policy with a `confidence` matrix maps (standard band rank, confidence)
//! to its own band. The standard band is the effective band computed under
//! the built-in `standard` preset; [`evaluate`] supplies it through the
//! crate-private [`evaluate_with`] hook, together with the built-in action
//! catalog.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::confidence::residual_confidence;
use crate::model::{
    Action, Band, Conditions, Confidence, DispositionKind, Matrix, Policy, Precedence,
};
use crate::subject::{Subject, SubjectFm};
use crate::{LoadedPolicy, PolicyError, PolicySource, invalid, parse_days};

/// The key of the optional confidence matrix.
const CONFIDENCE_KEY: &str = "confidence";

/// A calendar date, as days since 1970-01-01.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(pub u32);

impl fmt::Display for Date {
    /// `YYYY-MM-DD`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = civil_from_days(i64::from(self.0));
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

/// Parse `YYYY-MM-DD`, or an RFC 3339 timestamp
/// (`YYYY-MM-DDThh:mm:ss[.frac](Z|±hh:mm)`, `T` or a space as separator),
/// taking its date part. Anything else after the date is rejected, as are
/// dates before 1970-01-01.
///
/// Both `as_of` and an acceptance's `at` are treated as **UTC calendar
/// dates**: the time and offset of a timestamp are checked for syntax and
/// then ignored (the server stamps `at` in UTC).
pub fn parse_date(s: &str) -> Result<Date, PolicyError> {
    let bad = || {
        invalid(
            "date",
            format!("`{s}` is not a date of the form YYYY-MM-DD"),
        )
    };
    let bytes = s.as_bytes();
    if bytes.len() < 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes.len() > 10
            && !(matches!(bytes[10], b'T' | b't' | b' ') && is_rfc3339_time(&bytes[11..])))
    {
        return Err(bad());
    }
    let number = |range: std::ops::Range<usize>| -> Result<u32, PolicyError> {
        let part = &bytes[range];
        if !part.iter().all(u8::is_ascii_digit) {
            return Err(bad());
        }
        Ok(part.iter().fold(0u32, |n, b| n * 10 + u32::from(b - b'0')))
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    if year < 1970 || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(bad());
    }
    let days = days_from_civil(i64::from(year), month, day);
    u32::try_from(days).map(Date).map_err(|_| bad())
}

/// `hh:mm:ss[.frac](Z|z|±hh:mm)`: a syntax check only.
fn is_rfc3339_time(t: &[u8]) -> bool {
    let digits =
        |r: std::ops::Range<usize>| t.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    if !(digits(0..2)
        && t.get(2) == Some(&b':')
        && digits(3..5)
        && t.get(5) == Some(&b':')
        && digits(6..8))
    {
        return false;
    }
    let mut rest = &t[8..];
    if let Some(frac) = rest.strip_prefix(b".") {
        let n = frac.iter().take_while(|b| b.is_ascii_digit()).count();
        if n == 0 {
            return false;
        }
        rest = &frac[n..];
    }
    match rest {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => [h1, h2, m1, m2].iter().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The evaluation result.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Suggestions {
    /// The policy's identity.
    pub policy: PolicyRef,
    /// The `as_of` date, `YYYY-MM-DD`.
    pub as_of: String,
    /// One entry per non-excluded failure mode, in id order.
    pub failure_modes: Vec<FmSuggestion>,
    /// Excluded failure modes, in id order.
    pub excluded: Vec<ExcludedFm>,
    /// Analysis-level counts and lists.
    pub rollups: Rollups,
    /// The policy-relative sign-off verdict.
    pub signoff: SignoffVerdict,
}

/// Which policy produced the suggestions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PolicyRef {
    /// Policy name.
    pub name: String,
    /// Policy hash ([`policy_hash`](crate::policy_hash)).
    pub hash: String,
    /// Policy class.
    pub class: String,
    /// `derived_from` of the matrix that produced the effective bands: the
    /// strategy's S×P matrix, or `standard-bands` for a confidence matrix.
    pub matrix_derived_from: String,
    /// Where the policy came from.
    pub source: PolicySource,
}

/// The suggestions for one failure mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FmSuggestion {
    /// Failure-mode id.
    pub id: String,
    /// The unmitigated band; `None` when unscored.
    pub inherent_band: Option<String>,
    /// The residual band; `None` when unscored.
    pub residual_band: Option<String>,
    /// The band the policy acts on; `None` when unscored.
    pub effective_band: Option<String>,
    /// Confidence in the residual.
    pub confidence: Confidence,
    /// Present when low confidence moved the effective band.
    pub fallback: Option<Fallback>,
    /// Recorded or suggested disposition.
    pub status: DispositionStatus,
    /// Leaving this failure mode without action needs a written rationale.
    pub rationale_needed: bool,
    /// Suggested actions, by precedence then id.
    pub actions: Vec<ActionSuggestion>,
}

/// A fallback from the residual band to the (worse) inherent band.
///
/// Under a confidence-view policy (one with a `confidence` matrix), `from`
/// and `to` name the bands of the `standard` preset, whose effective band
/// selects the confidence-matrix row, not the policy's own bands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Fallback {
    /// The residual band.
    pub from: String,
    /// The inherent band, now effective.
    pub to: String,
    /// Why, naming the confidence and the threshold.
    pub reason: String,
}

/// A failure mode's disposition: recorded (accepted) or suggested.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DispositionStatus {
    /// Not yet scored.
    Unscored {
        /// Always `research`.
        suggested: DispositionKind,
    },
    /// Open: a disposition is suggested.
    Open {
        /// The suggested disposition.
        suggested: DispositionKind,
    },
    /// The risk was accepted.
    Accepted {
        /// Who accepted it.
        approver: String,
        /// When (RFC 3339).
        at: Option<String>,
        /// The acceptance is older than the effective band's
        /// `review_interval` (or its date is unknown).
        review_due: bool,
        /// The effective band's suggested acceptance role.
        acceptance_role: Option<String>,
        /// The roles that must concur.
        concurrence: Vec<String>,
    },
}

/// A suggested action for one failure mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionSuggestion {
    /// Action id.
    pub id: String,
    /// What to do.
    pub text: String,
    /// The action's tags.
    pub tags: Vec<String>,
    /// The action's precedence.
    pub precedence: Precedence,
    /// `required_when` holds.
    pub required: bool,
    /// The facts that matched `applies_when` (and `required_when`, prefixed
    /// `required: `).
    pub why: Vec<String>,
    /// Active mitigations that already satisfy the action, or
    /// [`ACCEPTANCE`] (`"acceptance"`) for a required `eliminate` /
    /// `procedure` action satisfied by an allowed, reasoned acceptance.
    pub covered_by: Vec<String>,
}

/// An excluded failure mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExcludedFm {
    /// Failure-mode id.
    pub id: String,
    /// The recorded exclusion rationale.
    pub rationale: String,
}

/// Analysis-level counts and lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Rollups {
    /// Failure modes by effective band (`unscored` for none).
    pub by_band: BTreeMap<String, usize>,
    /// Failure modes by confidence.
    pub by_confidence: BTreeMap<Confidence, usize>,
    /// Failure modes by status (`unscored`, `open`, `accepted`).
    pub by_status: BTreeMap<String, usize>,
    /// Required actions with no `covered_by`: `(failure mode id, action id)`.
    pub required_uncovered: Vec<(String, String)>,
    /// Accepted risks by suggested acceptance role (`unassigned` for none).
    pub accepted_by_role: BTreeMap<String, usize>,
    /// Accepted failure modes whose review is due.
    pub review_overdue: Vec<String>,
    /// Failure modes whose effective band fell back.
    pub fallbacks: Vec<String>,
}

/// The policy-relative sign-off verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SignoffVerdict {
    /// `reasons` is empty.
    pub pass: bool,
    /// Each failing rule, naming the failure mode (or `kernel: <blocker>`).
    pub reasons: Vec<String>,
}

/// How [`group`] groups suggestions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    /// Failure-mode id → its action ids.
    FailureMode,
    /// Effective band (`unscored` for none) → failure-mode ids.
    Band,
    /// Action tag → failure-mode ids.
    Tag,
    /// Action id → failure-mode ids.
    Action,
}

/// Group evaluated suggestions. Values are in failure-mode id order (or
/// action order for [`GroupBy::FailureMode`]) without duplicates.
pub fn group(s: &Suggestions, by: GroupBy) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    fn add(groups: &mut BTreeMap<String, Vec<String>>, key: &str, value: &str) {
        let values = groups.entry(key.to_string()).or_default();
        if !values.iter().any(|v| v == value) {
            values.push(value.to_string());
        }
    }
    for fm in &s.failure_modes {
        match by {
            GroupBy::FailureMode => {
                // Every failure mode appears, even with no actions.
                groups.entry(fm.id.clone()).or_default();
                for action in &fm.actions {
                    add(&mut groups, &fm.id, &action.id);
                }
            }
            GroupBy::Band => add(
                &mut groups,
                fm.effective_band.as_deref().unwrap_or(UNSCORED),
                &fm.id,
            ),
            GroupBy::Tag => {
                for tag in fm.actions.iter().flat_map(|a| &a.tags) {
                    add(&mut groups, tag, &fm.id);
                }
            }
            GroupBy::Action => {
                for action in &fm.actions {
                    add(&mut groups, &action.id, &fm.id);
                }
            }
        }
    }
    groups
}

const UNSCORED: &str = "unscored";

/// Evaluate `subject` against `policy` as of `as_of` (spec §4).
///
/// Errors: [`PolicyError::StrategyMismatch`] when the policy has no matrix for
/// the subject's strategy; [`PolicyError::Invalid`] only for a policy that
/// was not [validated](crate::validate()).
///
/// Built-in actions (`include_builtin_actions`) come from the shared catalog,
/// and a `confidence` matrix reads its rows from the built-in `standard`
/// preset, both supplied through the crate-private `evaluate_with`.
pub fn evaluate(
    subject: &Subject,
    policy: &LoadedPolicy,
    as_of: Date,
) -> Result<Suggestions, PolicyError> {
    let standard = crate::presets::standard_policy();
    let builtin = crate::presets::builtin_actions();
    evaluate_with(subject, policy, standard.as_ref(), &builtin, as_of)
}

/// [`evaluate`] with the `standard` preset (for confidence matrices; `None`
/// uses the policy itself) and the built-in action catalog.
pub(crate) fn evaluate_with(
    subject: &Subject,
    loaded: &LoadedPolicy,
    standard: Option<&Policy>,
    builtin_actions: &[Action],
    as_of: Date,
) -> Result<Suggestions, PolicyError> {
    let policy = &loaded.policy;
    let strategy = subject.strategy.id();
    let sxp = sxp_matrix(policy, strategy)?;
    let confidence_view = match policy.matrices.get(CONFIDENCE_KEY) {
        Some(matrix) => {
            let standard = standard.unwrap_or(policy);
            Some((matrix, standard, sxp_matrix(standard, strategy)?))
        }
        None => None,
    };
    let catalog = catalog(policy, builtin_actions);
    let max_band = band(policy, &policy.signoff.max_residual_band)?;

    let mut fms: Vec<&SubjectFm> = subject.failure_modes.iter().collect();
    fms.sort_by(|a, b| a.id.cmp(&b.id));

    let mut out = Vec::new();
    let mut excluded = Vec::new();
    let mut rollups = Rollups::default();
    let mut reasons = Vec::new();
    for fm in fms {
        if fm.disposition == fmeca::Disposition::Excluded {
            excluded.push(ExcludedFm {
                id: fm.id.clone(),
                rationale: fm.disposition_rationale.clone().unwrap_or_default(),
            });
            continue;
        }
        let confidence = residual_confidence(fm);
        let own = bands(policy, sxp, fm, confidence)?;
        let banded = match confidence_view {
            Some((matrix, standard, standard_sxp)) => {
                let std = bands(standard, standard_sxp, fm, confidence)?;
                let effective = match std.effective {
                    Some(b) => Some(confidence_cell(policy, standard, matrix, b, confidence)?),
                    None => None,
                };
                Banded {
                    effective,
                    fallback: std.fallback,
                    ..own
                }
            }
            None => own,
        };
        let suggestion = suggest(subject, policy, &catalog, fm, confidence, banded, as_of)?;

        // Rollups and sign-off.
        let band_key = suggestion.effective_band.as_deref().unwrap_or(UNSCORED);
        *rollups.by_band.entry(band_key.to_string()).or_default() += 1;
        *rollups.by_confidence.entry(confidence).or_default() += 1;
        if suggestion.fallback.is_some() {
            rollups.fallbacks.push(fm.id.clone());
        }
        let mut accepted_ok = false;
        let mut overdue = false;
        // An acceptance counts only in a band whose dispositions allow it.
        let mut acceptance_not_allowed = false;
        let status_key = match &suggestion.status {
            DispositionStatus::Unscored { .. } => "unscored",
            DispositionStatus::Open { .. } => "open",
            DispositionStatus::Accepted {
                review_due,
                acceptance_role,
                ..
            } => {
                let role = acceptance_role.as_deref().unwrap_or("unassigned");
                *rollups
                    .accepted_by_role
                    .entry(role.to_string())
                    .or_default() += 1;
                if *review_due {
                    rollups.review_overdue.push(fm.id.clone());
                }
                // "Overdue" is the reason only when the acceptance would
                // otherwise have counted as a pass.
                acceptance_not_allowed = policy.signoff.accepted_counts_as_pass
                    && banded
                        .effective
                        .is_some_and(|b| !b.dispositions.contains(&DispositionKind::Accept));
                overdue = policy.signoff.accepted_counts_as_pass
                    && *review_due
                    && policy.signoff.accepted_requires_review_current;
                accepted_ok =
                    policy.signoff.accepted_counts_as_pass && !overdue && !acceptance_not_allowed;
                "accepted"
            }
        };
        *rollups.by_status.entry(status_key.to_string()).or_default() += 1;
        if fm.inherent.is_none() {
            reasons.push(format!("{}: unscored", fm.id));
        }
        if let Some(effective) = banded.effective
            && effective.rank > max_band.rank
            && !accepted_ok
        {
            reasons.push(if acceptance_not_allowed {
                format!(
                    "{}: accepted in band '{}' which does not allow acceptance",
                    fm.id, effective.id
                )
            } else if overdue {
                format!(
                    "{}: accepted risk in band `{}` is overdue for review",
                    fm.id, effective.id
                )
            } else {
                format!(
                    "{}: effective band `{}` ranks above max_residual_band `{}`",
                    fm.id, effective.id, max_band.id
                )
            });
        }
        for action in &suggestion.actions {
            if action.required && action.covered_by.is_empty() {
                rollups
                    .required_uncovered
                    .push((fm.id.clone(), action.id.clone()));
                reasons.push(format!(
                    "{}: required action `{}` is not covered",
                    fm.id, action.id
                ));
            }
        }
        out.push(suggestion);
    }
    if policy.signoff.include_kernel_gate {
        reasons.extend(
            subject
                .kernel_gate
                .blockers
                .iter()
                .map(|b| format!("kernel: {b}")),
        );
    }

    Ok(Suggestions {
        policy: PolicyRef {
            name: policy.name.clone(),
            hash: loaded.hash.clone(),
            class: policy.class.clone(),
            matrix_derived_from: match confidence_view {
                Some((matrix, ..)) => matrix.derived_from.clone(),
                None => sxp.derived_from.clone(),
            },
            source: loaded.source.clone(),
        },
        as_of: as_of.to_string(),
        failure_modes: out,
        excluded,
        rollups,
        signoff: SignoffVerdict {
            pass: reasons.is_empty(),
            reasons,
        },
    })
}

/// The policy's S×P matrix for `strategy`, or a strategy mismatch.
fn sxp_matrix<'p>(policy: &'p Policy, strategy: &str) -> Result<&'p Matrix, PolicyError> {
    policy.matrices.get(strategy).ok_or_else(|| {
        PolicyError::StrategyMismatch(format!(
            "policy `{}` has no `{strategy}` matrix",
            policy.name
        ))
    })
}

/// The policy's actions, then the built-in ones it does not override, when
/// `include_builtin_actions`; sorted by precedence then id.
fn catalog<'p>(policy: &'p Policy, builtin: &'p [Action]) -> Vec<&'p Action> {
    let mut actions: Vec<&Action> = policy.actions.iter().collect();
    if policy.include_builtin_actions {
        actions.extend(
            builtin
                .iter()
                .filter(|b| !policy.actions.iter().any(|a| a.id == b.id)),
        );
    }
    actions.sort_by(|a, b| (a.precedence, &a.id).cmp(&(b.precedence, &b.id)));
    actions
}

fn band<'p>(policy: &'p Policy, id: &str) -> Result<&'p Band, PolicyError> {
    policy
        .bands
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| invalid("bands", format!("unknown band `{id}`")))
}

/// The band in `matrix` for `(severity, probability)` ordinals. Ordinals are
/// clamped to the matrix as the kernel clamps them.
fn sxp_cell<'p>(
    policy: &'p Policy,
    matrix: &Matrix,
    (severity, probability): (u8, u8),
) -> Result<&'p Band, PolicyError> {
    let at = |ordinal: u8, len: usize| usize::from(ordinal.max(1) - 1).min(len.saturating_sub(1));
    let row = matrix
        .cells
        .get(at(severity, matrix.cells.len()))
        .ok_or_else(|| invalid("matrices", "empty matrix"))?;
    let id = row
        .get(at(probability, row.len()))
        .ok_or_else(|| invalid("matrices", "empty matrix row"))?;
    band(policy, id)
}

/// The confidence-matrix band for a `standard` band at `confidence`: row is
/// the standard band's position in rank order, column `C4`…`C0`.
fn confidence_cell<'p>(
    policy: &'p Policy,
    standard: &Policy,
    matrix: &Matrix,
    standard_band: &Band,
    confidence: Confidence,
) -> Result<&'p Band, PolicyError> {
    let row = standard
        .bands
        .iter()
        .filter(|b| b.rank < standard_band.rank)
        .count()
        .min(matrix.cells.len().saturating_sub(1));
    let column = Confidence::C4 as usize - confidence as usize;
    let id = matrix
        .cells
        .get(row)
        .and_then(|r| r.get(column))
        .ok_or_else(|| invalid("matrices.confidence.cells", "missing cell"))?;
    band(policy, id)
}

/// A failure mode's bands under one policy.
#[derive(Clone, Copy)]
struct Banded<'p> {
    inherent: Option<&'p Band>,
    residual: Option<&'p Band>,
    effective: Option<&'p Band>,
    /// Confidence is below the residual's required confidence
    /// ([`required_confidence`]).
    low_confidence: bool,
    fallback: Option<FallbackRef<'p>>,
}

/// A fallback, by band reference (rendered into [`Fallback`] later).
#[derive(Clone, Copy)]
struct FallbackRef<'p> {
    from: &'p Band,
    to: &'p Band,
    /// The required confidence ([`required_confidence`]).
    threshold: Confidence,
    /// The band whose `min_confidence` set `threshold`.
    needed_by: &'p Band,
}

/// The confidence a residual needs to count (rule 4; 0.2 final review
/// ruling): the highest `min_confidence` over the residual band itself and
/// every band the claimed reduction leaves — those ranked above the residual
/// band and at or below the inherent band — with the band that sets it
/// (the highest-ranked on a tie). Claiming to leave a band needs that band's
/// evidence, so a bigger claimed reduction never needs less. `None` when none
/// of those bands sets a `min_confidence`.
fn required_confidence<'p>(
    policy: &'p Policy,
    inherent: &Band,
    residual: &'p Band,
) -> Option<(Confidence, &'p Band)> {
    policy
        .bands
        .iter()
        .filter(|b| b.id == residual.id || (b.rank > residual.rank && b.rank <= inherent.rank))
        .filter_map(|b| b.min_confidence.map(|m| (m, b)))
        .max_by_key(|(m, b)| (*m, b.rank))
}

/// Rules 1, 2 and 4 under `policy`'s S×P `matrix`.
fn bands<'p>(
    policy: &'p Policy,
    matrix: &Matrix,
    fm: &SubjectFm,
    confidence: Confidence,
) -> Result<Banded<'p>, PolicyError> {
    let Some(inherent_cell) = fm.inherent else {
        return Ok(Banded {
            inherent: None,
            residual: None,
            effective: None,
            low_confidence: false,
            fallback: None,
        });
    };
    let inherent = sxp_cell(policy, matrix, inherent_cell)?;
    let residual = match fm.residual {
        Some(cell) => sxp_cell(policy, matrix, cell)?,
        None => inherent,
    };
    let required = required_confidence(policy, inherent, residual).filter(|r| confidence < r.0);
    let low_confidence = fm.residual.is_some() && required.is_some();
    let fallback = match required {
        // Fall back only to a worse inherent band: low confidence never
        // lowers the band (a residual can be worse than the inherent value,
        // e.g. after a rescore down).
        Some((threshold, needed_by)) if low_confidence && inherent.rank > residual.rank => {
            Some(FallbackRef {
                from: residual,
                to: inherent,
                threshold,
                needed_by,
            })
        }
        _ => None,
    };
    Ok(Banded {
        inherent: Some(inherent),
        residual: Some(residual),
        effective: Some(if fallback.is_some() {
            inherent
        } else {
            residual
        }),
        low_confidence,
        fallback,
    })
}

/// Rules 5 and 6 for one failure mode.
fn suggest(
    subject: &Subject,
    policy: &Policy,
    catalog: &[&Action],
    fm: &SubjectFm,
    confidence: Confidence,
    banded: Banded<'_>,
    as_of: Date,
) -> Result<FmSuggestion, PolicyError> {
    let effective = banded.effective;
    let has_mitigation = fm.mitigations.iter().any(|m| m.is_active());
    let is_accepted = fm.disposition == fmeca::Disposition::Accepted;

    let status = if is_accepted {
        let accepted = fm.accepted.as_ref();
        let at = accepted.and_then(|a| a.at.clone());
        DispositionStatus::Accepted {
            approver: accepted.map(|a| a.approver.clone()).unwrap_or_default(),
            review_due: review_due(effective, at.as_deref(), as_of)?,
            at,
            acceptance_role: effective.and_then(|b| b.acceptance.clone()),
            concurrence: effective.map(|b| b.concurrence.clone()).unwrap_or_default(),
        }
    } else {
        match effective {
            None => DispositionStatus::Unscored {
                suggested: DispositionKind::Research,
            },
            Some(band) => DispositionStatus::Open {
                suggested: suggested_disposition(band, banded.low_confidence),
            },
        }
    };

    let facts = Facts {
        policy,
        subject,
        fm,
        effective,
        confidence,
        fallback: banded.fallback.is_some(),
    };
    let mut actions = Vec::new();
    for action in catalog {
        let Some(why) = facts.check(&action.applies_when)? else {
            continue;
        };
        let required = match &action.required_when {
            Some(conditions) => facts.check(conditions)?,
            None => None,
        };
        let mut why = why;
        if let Some(required_why) = &required {
            why.extend(required_why.iter().map(|w| format!("required: {w}")));
        }
        actions.push(ActionSuggestion {
            id: action.id.clone(),
            text: action.text.clone(),
            tags: action.tags.clone(),
            precedence: action.precedence,
            required: required.is_some(),
            why,
            covered_by: if required.is_some() && acceptance_covers(action.precedence, fm, effective)
            {
                vec![ACCEPTANCE.to_string()]
            } else {
                covered_by(action.precedence, fm)
            },
        });
    }

    Ok(FmSuggestion {
        id: fm.id.clone(),
        inherent_band: banded.inherent.map(|b| b.id.clone()),
        residual_band: banded.residual.map(|b| b.id.clone()),
        effective_band: effective.map(|b| b.id.clone()),
        confidence,
        fallback: banded.fallback.map(|f| Fallback {
            from: f.from.id.clone(),
            to: f.to.id.clone(),
            reason: format!(
                "confidence {confidence:?} is below `{}`'s min_confidence {:?}, \
                 needed to claim `{}` → `{}`",
                f.needed_by.id, f.threshold, f.to.id, f.from.id
            ),
        }),
        status,
        rationale_needed: effective.is_some_and(|b| b.no_action_needs_rationale)
            && !has_mitigation
            && !is_accepted,
        actions,
    })
}

/// `as_of − at > review_interval` for the effective band. No band or no
/// interval: never due. An acceptance with no (or an unreadable) date is due:
/// it cannot be shown current.
fn review_due(band: Option<&Band>, at: Option<&str>, as_of: Date) -> Result<bool, PolicyError> {
    let Some(interval) = band.and_then(|b| b.review_interval.as_deref()) else {
        return Ok(false);
    };
    let interval = parse_days(interval)?;
    Ok(match at.map(parse_date) {
        Some(Ok(at)) => as_of.0.saturating_sub(at.0) > interval,
        _ => true,
    })
}

/// Rule 5's order: `research` when allowed and the cause is low confidence;
/// else `mitigate`; else `watch`; else the band's first disposition.
fn suggested_disposition(band: &Band, low_confidence: bool) -> DispositionKind {
    let allows = |d: DispositionKind| band.dispositions.contains(&d);
    if low_confidence && allows(DispositionKind::Research) {
        DispositionKind::Research
    } else if allows(DispositionKind::Mitigate) {
        DispositionKind::Mitigate
    } else if allows(DispositionKind::Watch) {
        DispositionKind::Watch
    } else {
        band.dispositions
            .first()
            .copied()
            .unwrap_or(DispositionKind::Research)
    }
}

/// The `covered_by` marker for a required action satisfied by an acceptance.
pub const ACCEPTANCE: &str = "acceptance";

/// A required `eliminate` or `procedure` action, which no mitigation kind
/// covers, is satisfied by a recorded acceptance with a non-empty rationale in
/// an effective band whose dispositions allow `accept`.
fn acceptance_covers(precedence: Precedence, fm: &SubjectFm, effective: Option<&Band>) -> bool {
    matches!(precedence, Precedence::Eliminate | Precedence::Procedure)
        && fm.disposition == fmeca::Disposition::Accepted
        && fm
            .accepted
            .as_ref()
            .is_some_and(|a| !a.rationale.trim().is_empty())
        && effective.is_some_and(|b| b.dispositions.contains(&DispositionKind::Accept))
}

/// Implemented or verified mitigations of the kind that satisfies
/// `precedence`: prevent → prevention, detect → detection, contain →
/// fail-fast. Planned and abandoned mitigations do not cover; eliminate and
/// procedure are never auto-covered.
fn covered_by(precedence: Precedence, fm: &SubjectFm) -> Vec<String> {
    let kind = match precedence {
        Precedence::Prevent => fmeca::MitigationKind::Prevention,
        Precedence::Detect => fmeca::MitigationKind::Detection,
        Precedence::Contain => fmeca::MitigationKind::FailFast,
        Precedence::Eliminate | Precedence::Procedure => return Vec::new(),
    };
    fm.mitigations
        .iter()
        .filter(|m| {
            m.kind == kind
                && matches!(
                    m.status,
                    fmeca::MitigationStatus::Implemented | fmeca::MitigationStatus::Verified
                )
        })
        .map(|m| m.id.clone())
        .collect()
}

/// The facts conditions are checked against.
struct Facts<'a> {
    policy: &'a Policy,
    subject: &'a Subject,
    fm: &'a SubjectFm,
    effective: Option<&'a Band>,
    confidence: Confidence,
    fallback: bool,
}

impl Facts<'_> {
    /// `Some(facts that matched)` when every present condition holds.
    fn check(&self, c: &Conditions) -> Result<Option<Vec<String>>, PolicyError> {
        let mut why = Vec::new();
        macro_rules! require {
            ($holds:expr, $fact:expr) => {
                if !$holds {
                    return Ok(None);
                }
                why.push($fact);
            };
        }
        if let Some(at_least) = &c.band_at_least {
            let threshold = band(self.policy, at_least)?;
            let Some(effective) = self.effective else {
                return Ok(None);
            };
            require!(
                effective.rank >= threshold.rank,
                format!("band {} ≥ {}", effective.id, threshold.id)
            );
        }
        if let Some(at_least) = c.rank_at_least {
            let Some(effective) = self.effective else {
                return Ok(None);
            };
            require!(
                effective.rank >= at_least,
                format!("band {} rank {} ≥ {at_least}", effective.id, effective.rank)
            );
        }
        if let Some(bands) = &c.band_in {
            let Some(effective) = self.effective else {
                return Ok(None);
            };
            require!(
                bands.contains(&effective.id),
                format!("band {}", effective.id)
            );
        }
        if let Some(levels) = &c.level_in {
            let Some(level) = self.subject.level else {
                return Ok(None);
            };
            require!(levels.contains(&level), format!("level {}", name(&level)));
        }
        if let Some(domains) = &c.domain_in {
            require!(
                domains.contains(&self.fm.domain),
                format!("domain {}", name(&self.fm.domain))
            );
        }
        if let Some(classes) = &c.response_class_in {
            let Some(class) = self.fm.response_class else {
                return Ok(None);
            };
            require!(
                classes.contains(&class),
                format!("response_class {}", name(&class))
            );
        }
        if let Some(single_point) = c.single_point {
            require!(
                self.fm.single_point == single_point,
                format!("single_point {single_point}")
            );
        }
        if let Some(below) = c.confidence_below {
            require!(
                self.confidence < below,
                format!("confidence {:?} < {below:?}", self.confidence)
            );
        }
        if let Some(at_least) = c.confidence_at_least {
            require!(
                self.confidence >= at_least,
                format!("confidence {:?} ≥ {at_least:?}", self.confidence)
            );
        }
        if let Some(dispositions) = &c.disposition_in {
            require!(
                dispositions.contains(&self.fm.disposition),
                format!("disposition {}", name(&self.fm.disposition))
            );
        }
        if let Some(fallback) = c.fallback {
            require!(self.fallback == fallback, format!("fallback {fallback}"));
        }
        if let Some(classes) = &c.class_in {
            require!(
                classes.contains(&self.policy.class),
                format!("class {}", self.policy.class)
            );
        }
        if let Some(kinds) = &c.mitigation_kinds_missing {
            let missing: Vec<String> = kinds.iter().map(name).collect();
            require!(
                !self
                    .fm
                    .mitigations
                    .iter()
                    .any(|m| m.is_active() && kinds.contains(&m.kind)),
                format!("no {} mitigation", missing.join("/"))
            );
        }
        if let Some(kinds) = &c.mitigation_kinds_present {
            let present: Vec<String> = self
                .fm
                .mitigations
                .iter()
                .filter(|m| m.is_active() && kinds.contains(&m.kind))
                .map(|m| name(&m.kind))
                .collect();
            require!(
                !present.is_empty(),
                format!("{} mitigation present", present.join("/"))
            );
        }
        Ok(Some(why))
    }
}

/// The serde (snake_case) name of a unit enum value.
fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
