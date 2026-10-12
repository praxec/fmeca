//! Markdown rendering of [`Suggestions`] (plan Task 5, spec §5).
//!
//! The output is deterministic: bands in descending rank order from `policy`,
//! failure modes in id order, actions with the required ones first, excluded
//! failure modes, then the sign-off verdict and its reasons.

use std::fmt::Write;

use crate::model::Policy;
use crate::{ActionSuggestion, PolicySource};
use crate::{DispositionKind, DispositionStatus, FmSuggestion, Precedence, Suggestions};

/// Render suggestions as deterministic Markdown, ordering band sections by
/// `policy`'s [`rank`](crate::Band::rank).
pub fn to_markdown(s: &Suggestions, policy: &Policy) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Risk policy suggestions — {}", s.policy.name);
    let _ = writeln!(out);
    let _ = writeln!(out, "- Policy: `{}`", s.policy.name);
    let _ = writeln!(out, "- Class: `{}`", s.policy.class);
    let _ = writeln!(out, "- Hash: `{}`", s.policy.hash);
    let _ = writeln!(out, "- Source: {}", source_name(&s.policy.source));
    let _ = writeln!(out, "- Matrix: `{}`", s.policy.matrix_derived_from);
    let _ = writeln!(out, "- As of: `{}`", s.as_of);
    let _ = writeln!(out);

    for band in bands_in_rank_order(s, policy) {
        let _ = writeln!(out, "## {band}");
        let _ = writeln!(out);
        for fm in s
            .failure_modes
            .iter()
            .filter(|fm| fm.effective_band.as_deref() == Some(band.as_str()))
        {
            render_fm(&mut out, fm);
        }
    }

    let unscored: Vec<&FmSuggestion> = s
        .failure_modes
        .iter()
        .filter(|fm| fm.effective_band.is_none())
        .collect();
    if !unscored.is_empty() {
        let _ = writeln!(out, "## unscored");
        let _ = writeln!(out);
        for fm in unscored {
            render_fm(&mut out, fm);
        }
    }

    if !s.excluded.is_empty() {
        let _ = writeln!(out, "## Excluded");
        let _ = writeln!(out);
        for fm in &s.excluded {
            let _ = writeln!(out, "- `{}`: {}", fm.id, fm.rationale);
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Sign-off");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Verdict: {}",
        if s.signoff.pass { "pass" } else { "fail" }
    );
    for reason in &s.signoff.reasons {
        let _ = writeln!(out, "- Reason: {reason}");
    }
    out
}

/// The band ids present, highest `policy` rank first (then id ascending).
fn bands_in_rank_order(s: &Suggestions, policy: &Policy) -> Vec<String> {
    let mut present: Vec<String> = Vec::new();
    for fm in &s.failure_modes {
        if let Some(band) = &fm.effective_band
            && !present.contains(band)
        {
            present.push(band.clone());
        }
    }
    let rank = |id: &str| {
        policy
            .bands
            .iter()
            .find(|band| band.id == id)
            .map(|band| band.rank)
    };
    present.sort_by(|a, b| rank(b).cmp(&rank(a)).then_with(|| a.cmp(b)));
    present
}

fn render_fm(out: &mut String, fm: &FmSuggestion) {
    let _ = writeln!(out, "### {}", fm.id);
    let _ = writeln!(out);
    let _ = writeln!(out, "- confidence: {}", confidence_name(fm.confidence));
    if let Some(fallback) = &fm.fallback {
        let _ = writeln!(
            out,
            "- fallback: `{}` → `{}` ({})",
            fallback.from, fallback.to, fallback.reason
        );
    }
    let _ = writeln!(out, "- status: {}", status_name(&fm.status));
    if fm.rationale_needed {
        let _ = writeln!(out, "- rationale needed before leaving this without action");
    }
    if !fm.actions.is_empty() {
        let _ = writeln!(out, "- actions:");
        let mut actions: Vec<&ActionSuggestion> = fm.actions.iter().collect();
        actions.sort_by_key(|action| !action.required);
        for action in actions {
            render_action(out, action);
        }
    }
    let _ = writeln!(out);
}

fn render_action(out: &mut String, action: &ActionSuggestion) {
    let marker = if action.required { "[required] " } else { "" };
    let _ = writeln!(
        out,
        "  - {marker}`{}` ({}) — {}",
        action.id,
        precedence_name(action.precedence),
        action.text
    );
}

fn source_name(source: &PolicySource) -> &'static str {
    match source {
        PolicySource::Preset => "preset",
        PolicySource::Project => "project",
    }
}

fn confidence_name(confidence: crate::Confidence) -> String {
    format!("{confidence:?}")
}

fn status_name(status: &DispositionStatus) -> String {
    match status {
        DispositionStatus::Unscored { suggested } => {
            format!("unscored (suggested {})", kind(*suggested))
        }
        DispositionStatus::Open { suggested } => format!("open (suggested {})", kind(*suggested)),
        DispositionStatus::Accepted {
            approver,
            at,
            review_due,
            ..
        } => {
            let when = at.as_deref().unwrap_or("unknown");
            let overdue = if *review_due { ", review due" } else { "" };
            format!("accepted by {approver} at {when}{overdue}")
        }
    }
}

fn kind(kind: DispositionKind) -> &'static str {
    match kind {
        DispositionKind::Accept => "accept",
        DispositionKind::Mitigate => "mitigate",
        DispositionKind::Watch => "watch",
        DispositionKind::Research => "research",
        DispositionKind::Close => "close",
        DispositionKind::Elevate => "elevate",
    }
}

fn precedence_name(precedence: Precedence) -> &'static str {
    match precedence {
        Precedence::Eliminate => "eliminate",
        Precedence::Prevent => "prevent",
        Precedence::Detect => "detect",
        Precedence::Contain => "contain",
        Precedence::Procedure => "procedure",
    }
}
