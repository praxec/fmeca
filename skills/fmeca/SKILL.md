---
name: fmeca
description: "Use when you need the method reference behind the fmeca-* skills, or when no narrower fmeca skill fits a task that uses the fmeca-mcp tools (session.open, session.list, append, state.get, risk.next, readiness.assess, report.export, scoring.catalog, analyze, vet, actions.suggest, policy.list). Covers what an FMECA is, observations instead of scores, the 3x3 and NASA 5x5 matrices, the prevention, detection, fail_fast order, residual risk and standing, the ready and verified.ready gates, the .fmeca/ project store and its <aspect>-<level> session ids, parent links and refs between analyses, and the core loop that every level skill runs."
---

# FMECA with fmeca-mcp

An FMECA (failure modes, effects and criticality analysis) asks four questions of a thing:
how can it fail, what happens then, how bad and how likely is that, and what have we done
about it. fmeca-mcp keeps the ledger and does the arithmetic. You do the judgment: you name
failure modes, causes and effects, pick observations and propose mitigations.

You analyse one **level** at a time, one analysis per level, each linked to its parent:
`concept` → `conops` → `technical_plan` → `project_plan` → `implementation` →
`verification`. The level skills are [fmeca-concept](../fmeca-concept/SKILL.md) (`concept`
and `conops`), [fmeca-technical-plan](../fmeca-technical-plan/SKILL.md),
[fmeca-project-plan](../fmeca-project-plan/SKILL.md),
[fmeca-implementation](../fmeca-implementation/SKILL.md) and
[fmeca-verification](../fmeca-verification/SKILL.md). Then come
[fmeca-resolve](../fmeca-resolve/SKILL.md), [fmeca-vet](../fmeca-vet/SKILL.md),
[fmeca-policy](../fmeca-policy/SKILL.md) and [fmeca-signoff](../fmeca-signoff/SKILL.md). What each level reads and the link rules are in
[levels.md](levels.md). A one-shot input: [examples/one-shot-analyze.json](examples/one-shot-analyze.json).

## 1. The project store

- Run `fmeca-mcp init` once at the repo root. It creates `.fmeca/` with a README and a
  `.gitattributes` that union-merges the logs. **Commit the folder**: analyses are reviewed
  like code.
- Each analysis is one append-only log, `.fmeca/<session_id>.jsonl`. Never edit it by hand.
- The server uses `FMECA_STATE_DIR` if set, else the nearest `.fmeca/` above its working
  directory, else a per-user directory. Check `session.list.state_dir` is the project's
  `.fmeca/` before you write; otherwise the analysis lands outside the repo.
- **Find before you open.** Run `session.list` (filters `level`, `aspect`, `parent`) and
  resume an existing analysis rather than starting a second one for the same thing.
- **Session ids are `<aspect>-<level>`**, with the level's `_` written as `-`:
  `checkout-concept`, `checkout-technical-plan`. The aspect is what is analysed.
- Run `fmeca-mcp check` after every merge (and in CI). It prints `ok <id>` per log, or
  `error <id>: <reason>` and exits 1, for example on a duplicate id from a union merge.

## 2. Observations, not scores

You never supply a severity, probability or criticality. You supply **observations**: ids
from `scoring.catalog` for the session's matrix, such as `data_loss` (severity) or
`load_or_concurrency_dependent` (probability). The engine maps them to levels (the worst
observation on an axis wins) and the matrix gives criticality `low`, `medium` or `high`.

- `qualitative3x3` (default): three levels per axis. High severity with low probability is
  still `medium`, so a severe failure mode only reaches `low` when its effect is bounded.
- `nasa8004_5x5`: GSFC-HDBK-8004's five-level consequence and likelihood. Choose it with
  `session.open.matrix_strategy` for safety- or mission-critical work. Its ids differ.
- The strategy is fixed once the session is open. An unknown id is `INVALID_OBSERVATION`.
- Pick the observations the evidence supports, not the ones that give the answer you want.
- **Rescore** when the evidence changes. A rescore **replaces both axes**: resend the full
  `severity_observations` and `probability_observations`, including the ones that have not
  changed, or the axis you leave out is emptied and the failure mode becomes unscored.

```json
{"variant": "rescore", "session_id": "checkout-implementation", "rescore": {
  "failure_mode_id": "fm-retry-double-charge", "session_id": "checkout-implementation",
  "severity_observations": ["data_loss"], "probability_observations": ["occasional"],
  "source": {"turn_id": "vet-2026-10-11"}}}
```

## 3. Failure modes

Each failure mode has an `id` (unique in the session, shared with mitigation ids), a
`component` `{id}`, `description`, `cause`, `domain` (`ux`, `runtime`, `architecture`,
`delivery`, `security`), observations on both axes, and a `source` `{turn_id}` saying where
the fact came from. Add them with `append.add_failure_mode.failure_mode`. Also fill:
- **effects at three levels**: `local_effect` (here), `next_effect` (one level up) and
  `effect` (the end effect on users or the mission). With a `parent`, write `next_effect`
  as the parent failure mode's cause; that is how a lower analysis feeds a higher one;
- `single_point: true` when there is no redundancy or fallback. A single point with `high`
  raw criticality is a **critical item**, listed in `critical_items` until it is excluded;
- `scope` (`localized`, `cross_cutting`, `structural`) sharpens the computed
  `response_class` (`minor_fix`, `restructure`, `re_architecture`);
- `parent` `{session_id, failure_mode_id}` (the risk it refines) and `refs`.

A missing cause, effect or observation raises a clarify signal and blocks readiness.

## 4. Mitigations, in discipline order

Add with `append.add_mitigation.mitigation`: `id`, `failure_mode_id`, `kind`, `description`,
`residual_severity_observations`, `residual_probability_observations` and `source`.
1. `prevention`: the failure cannot happen (a type, a constraint, a removed dependency).
2. `detection`: it is caught before it does harm (a check, a test, an alert).
3. `fail_fast`: its effect is bounded when it happens (a timeout, a circuit breaker).

Prefer them in that order. A failure mode mitigated **only** by `fail_fast` raises
`weak_mitigation_order` and blocks readiness. The residual observations say what is still
true after the mitigation; give both axes, or the mitigation does not count.

## 5. Residual risk, standing and the gates

The residual criticality is the best residual among active (not `abandoned`) mitigations.
Standing follows: `unmitigated` (high or medium, no mitigation), `under_mitigated`
(mitigated, residual still high or medium) or `acceptable` (residual `low`).

`readiness.assess` has two gates:
- `ready`: every open failure mode is scored, none has residual high or medium, and no
  `weak_mitigation_order` stands. `blockers` says why not (`CLARIFY`, `RESIDUAL`,
  `DISCIPLINE`, `EMPTY`).
- `verified.ready`: the same gate counting only `verified` mitigations
  ([fmeca-resolve](../fmeca-resolve/SKILL.md)). Sign-off at `verification` needs it.

It also lists `accepted_risks`, `excluded`, `critical_items` and `resolution` counts.

## 6. Links

- `session.open.parent`: the parent analysis (it must exist), e.g. the conops above a
  technical plan. `session.list` with `parent` lists an analysis's children.
- Failure-mode `parent`: the higher-level failure mode this one refines.
- Failure-mode `refs` are anchors outside fmeca: `cpm:<plan_id>/<deliverable_id>`,
  `path:<file>`, `issue:<url>`, `pr:<url>`, `commit:<sha>`.

## 7. The core loop

Every level skill runs this loop:
1. `session.list`, then `session.open` with `session_id`, `level`, `aspect`, `title`,
   `scope_paths` and `parent`. Reopening an existing id returns it unchanged and ignores
   the metadata you pass; read `meta` in the response.
2. `scoring.catalog` for the session's `matrix_strategy`: the only observation ids you use.
3. `append` each failure mode you can name (`add_failure_mode`). Read the `signals` that
   come back and fix any clarify signal now.
4. `risk.next` returns the worst unmitigated failure mode, or `risk: null`.
5. `append` mitigations for it, prevention first (`add_mitigation`).
6. Repeat from 4 until `risk.next` is null. `risk.next` skips failure modes that are
   excluded or accepted (risk deliberately retained). Then run `readiness.assess` and work
   its `RESIDUAL` and `DISCIPLINE` blockers: under-mitigated failure modes never appear in
   `risk.next`.
7. Report the standing to the user and hand off to the next level.

For a quick, throwaway pass with no session, call `analyze` with the whole batch (see the
example). It uses the same computation and writes nothing.

## 8. Risk policies

The kernel gates above are fixed. A **risk policy** is a second, configurable reading of the
same analysis: the project's own risk bands, a sign-off rule and tagged actions to take.
`actions.suggest` computes it at query time and never writes to the log or changes a score;
`policy.list` shows the presets and the project's `.fmeca/policies/` files. Default
`compat-0.1` gives the same verdict as `readiness.assess`. See
[fmeca-policy](../fmeca-policy/SKILL.md) and [docs/policies.md](../../docs/policies.md).

## 9. vet is advisory

`vet` asks Jev, a calibrated judgment model, for probabilities about your observations,
mitigations, coverage and readiness. It never writes and never changes a score or a gate.
See [fmeca-vet](../fmeca-vet/SKILL.md).
