---
name: fmeca-policy
description: "Use when an fmeca analysis needs to be read under a project's own risk policy: the user asks which risk bands, sign-off rule or actions apply, wants stricter or looser rules than the default, or wants a custom policy written. Picks a preset (compat-0.1, prototype, standard, business-critical, safety-critical, confidence-view) with policy.list, runs the read-only actions.suggest, explains bands, confidence, fallback, dispositions and the sign-off verdict, and writes and validates a project policy in .fmeca/policies/ with fmeca-mcp policy validate. Suggests only; never changes the analysis."
---

# fmeca-policy: read an analysis under a risk policy

A **risk policy** re-colours an analysis into the project's own bands, sets a sign-off rule
and suggests tagged actions. It is applied at query time and never written to the log, so
one analysis can be read under several policies. The method is in
[the fmeca reference](../fmeca/SKILL.md); the model, every preset and the action catalog are
in [docs/policies.md](https://github.com/praxec/fmeca/blob/dev/docs/policies.md). If this skill and the server disagree, the
server wins.

## When to use

- The user asks "what do we have to fix under our policy", "is this good enough for
  production" or "what would safety-critical need".
- [fmeca-signoff](../fmeca-signoff/SKILL.md) needs the project's verdict.
- A team wants its own bands or actions.
- Not to change a score or a gate: that is `append` and `readiness.assess`.

## 1. Choose a policy

Run `policy.list`. It returns `presets` and `project`, each row `{name, hash, class,
description, source}`; a project file that fails to load carries an `error`. The presets:
`compat-0.1` (the default, equals the 0.1 `readiness.assess` verdict), `prototype`,
`standard`, `business-critical`, `safety-critical` and `confidence-view`. Project files are
`.fmeca/policies/<name>.toml` and resolve after the presets, so a file named like a preset
is shadowed. Ask the user which applies; do not pick a stricter one to get a better answer.
A name is a preset or a file stem, never a path.

## 2. Suggest

```json
{"session_id": "checkout-implementation", "policy": "standard", "group_by": "tag"}
```

`actions.suggest` is read-only: it never appends and never changes the analysis. Pass
exactly one of `actions.suggest.session_id` and `actions.suggest.analysis` (the `analyze`
envelope). Optional: `actions.suggest.as_of` (`YYYY-MM-DD`, default today, echoed back),
`actions.suggest.bands`, `actions.suggest.tags`, `actions.suggest.required_only` and
`actions.suggest.group_by` (`failure_mode`, `band`, `tag` or `action`). Filters narrow
`failure_modes`; `rollups` and `signoff` always cover the whole analysis.

One-shot `analysis` input (and `--analysis` in the CLI) has no level, no single-point
flags, no dispositions and no resolution evidence: every mitigation is planned, so every
residual is C0 and nothing is `covered_by`. Use it to screen; run sign-off on a session.

Read the result:
- `policy`: name and hash. Quote both when you report, so the verdict can be reproduced.
- per failure mode: `inherent_band`, `residual_band` and `effective_band`; `confidence`
  (C0 declared only, up to C4 proven in use, from the mitigations' evidence); `fallback`;
  `status`; `rationale_needed`; `actions`.
- `fallback`: when confidence is below the required minimum, the **inherent** band stands
  in for the residual. The minimum is the higher `min_confidence` of the residual band and
  of the first gated band the claimed reduction leaves, so a claim to the lowest band needs
  evidence even when that band asks for none. It only ever raises the band.
  A policy with `evidence_mode = "strong"` needs the evidence of every band it leaves. The cure is evidence, not a rescore:
  move the mitigation with [fmeca-resolve](../fmeca-resolve/SKILL.md) and cite the test or
  inspection.
- `status.state` is `unscored`, `open` or `accepted`. Unscored and open carry a `suggested`
  disposition (`accept`, `mitigate`, `watch`, `research`, `close`, `elevate`); it is a
  suggestion and the human decides. Accepted carries the `approver`, `review_due` and the
  acceptance and concurrence roles. An **unscored** failure mode fails policy sign-off (under
  `compat-0.1`, which sets `unscored_fails = false`, only while open, through the kernel's
  CLARIFY blocker).
- `actions` come with `tags`, `precedence` (eliminate, prevent, detect, contain,
  procedure), `required` and `covered_by`. A required action nothing covers is listed in
  `rollups.required_uncovered` and fails sign-off.
- `signoff.pass` and `signoff.reasons`: the policy verdict, not the kernel gate. Show them
  beside, never instead of, `readiness.assess`. Kernel blockers appear in the reasons as
  `kernel: ...` (every preset sets `include_kernel_gate`).
- Acceptance is band-gated: it counts as a pass only when the effective band's
  `dispositions` include `accept`; otherwise sign-off fails with "accepted in band ...
  which does not allow acceptance". The same rule covers a required `eliminate` or
  `procedure` action (`covered_by: ["acceptance"]`, with a non-empty rationale). See
  [docs/policies.md](https://github.com/praxec/fmeca/blob/dev/docs/policies.md).
- `excluded` failure modes are out of scope; `rollups.review_overdue` lists acceptances
  past their review date.

Errors, none of which fall back to another policy: `POLICY_NOT_FOUND`, `INVALID_POLICY`
(a typo in a band id, a non-monotone matrix, a bad `as_of`) and `POLICY_STRATEGY_MISMATCH`.

## 3. Write a project policy

1. Copy [policy-template.toml](policy-template.toml) (shipped with this skill: a complete,
   commented policy derived from `standard`) to `.fmeca/policies/<name>.toml` and set
   `name` to `<name>`, which matches `^[a-z0-9][a-z0-9-]{0,63}$`. Every preset's bands
   and cells are in [docs/policies.md](https://github.com/praxec/fmeca/blob/dev/docs/policies.md).
2. Give every band a unique `id` and `rank`; every matrix needs `derived_from` and must be
   monotone. Unknown keys are rejected.
3. Validate it, with the CLI: `fmeca-mcp policy validate .fmeca/policies/<name>.toml`
   prints `ok <name> <hash>` or the field that is wrong, and exits 2 on error. It also
   fails when `name` differs from the file stem or the stem is a preset name (shadowed).
4. Confirm with `policy.list` that it loads, then run `actions.suggest` with
   `actions.suggest.policy` set to the name.
5. Commit the file. Its hash covers the file as written; the built-in action catalog is
   merged at query time when `include_builtin_actions = true`.

## 4. In CI

`fmeca-mcp suggest <session-id> --policy <name>` (or `--analysis <file.json>`) prints JSON,
or markdown with `--format markdown`, and exits 0 when sign-off passes, 1 when it fails and
2 on a usage or policy error. `fmeca-mcp policy list` prints one row per policy:
`<source> <name> <hash> <class>`; a project file that fails to load (or is shadowed by a
preset) is still listed, as `project <stem> - - <ERROR>`.

## 5. Report

Give the policy name and hash, the sign-off verdict with its reasons, each fallback with
the evidence that would lift it, the required actions still uncovered and the acceptances
past review. Do not accept a risk or mark anything verified to change the verdict.
