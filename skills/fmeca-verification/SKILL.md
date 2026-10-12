---
name: fmeca-verification
description: Use when a change is about to ship or has shipped and the question is what has actually been proven, for test reports, CI runs, rollout, rollback and monitoring plans, or when the user asks whether a release is safe. Opens a verification analysis under the implementation with fmeca-mcp (session.open, scoring.catalog, append, risk.next, readiness.assess) covering untested paths, rollout and rollback, and observability gaps, with tests, alerts and staged rollout as mitigations verified by test results, CI run URLs and files, then hands off to fmeca-signoff.
---

# fmeca-verification: what did we prove

Analyse the evidence, not the intent, at the `verification` level: which failure paths are
tested, how the change rolls out and back, and whether anyone would notice it failing in
production. The method and the core loop are in [the fmeca reference](../fmeca/SKILL.md);
level conventions are in [levels.md](../fmeca/levels.md). If this skill and the server
disagree, the server wins.

## 1. When to use

- A change is ready to release, or has been released and is being watched.
- The implementation analysis is done and its mitigations need proof.
- The user asks "is this safe to ship", "what is untested" or "how do we roll this back".
- Not for: finding faults in code → use fmeca-implementation; the final gate → use
  fmeca-signoff.

## 2. Inputs to read

- the test reports and coverage for the changed code; which failure paths have a test;
- the CI runs for the release candidate;
- the rollout plan (flags, stages, canaries), the rollback plan and whether it was tried;
- the monitoring plan: alerts, dashboards, SLOs, who is paged;
- the parent implementation analysis (`state.get`): every mitigation that is
  `implemented` but not `verified` is a question this level must answer.

## 3. Session

```json
{"session_id": "checkout-verification", "title": "Checkout: release 2.4 verification",
 "level": "verification", "aspect": "checkout",
 "scope_paths": ["tests/payment_retry.rs", "docs/release/2.4-rollout.md"],
 "parent": "checkout-implementation"}
```

Then run the core loop: `scoring.catalog`, `append` failure modes, `risk.next`, `append`
mitigations in discipline order, until `risk.next` is null and the `readiness.assess`
blockers are worked.

## 4. What failure modes look like here

The `component` is the part of the release the gap is in: a test suite
(`tests.payment_retry`), the rollout (`release.rollout`), an alert (`ops.alerting`).

| Area | Typical failure modes | Domain |
|---|---|---|
| Untested paths | an error path or retry with no test; a test that cannot fail; a flaky test that is ignored | `runtime` |
| Rollout | no flag or stage, so every user gets it at once; a migration that cannot be undone | `delivery` |
| Rollback | a rollback that was never tried; a data change the old version cannot read | `delivery` |
| Observability | no alert on the failure; an alert nobody owns; a metric that hides the failure in an average | `runtime` |

- Fill all three effects. With a `parent` implementation failure mode, write
  `next_effect` as its cause: "the double-charge fault reaches production untested".
- Set `single_point` when one check is the only thing standing between the fault and
  users (one test, one alert, one person watching).

## 5. What counts as a mitigation, and what evidence verifies it

| Kind | At this level |
|---|---|
| `prevention` | a test that fails on the fault, run in CI; a migration made reversible |
| `detection` | an alert on the failure, with an owner; a canary that compares error rates |
| `fail_fast` | a staged rollout with a stop rule; a flag that turns the change off; a rehearsed rollback |

Evidence that verifies them (see [fmeca-resolve](../fmeca-resolve/SKILL.md)):
- the `test`, basis `test`, and a `url` to the CI run where it passed;
- the rollout or rollback record as `file` or `url`, basis `operational_use` once it has run;
- the alert definition as `file`, basis `inspection`, until it has fired in a drill.

Also go back to the parent analyses and move their mitigations to `verified` where this
evidence proves them.

To propose concrete work, call `actions.suggest` with `group_by: "tag"` (see
[fmeca-policy](../fmeca-policy/SKILL.md)); it only suggests.

## 6. Hand-off to the next level

- Hand off to [fmeca-signoff](../fmeca-signoff/SKILL.md). At this level sign-off needs
  `verified.ready`, not just `ready`.
- Optionally run [fmeca-vet](../fmeca-vet/SKILL.md) first for a second opinion.
- Report to the user: what is proven, what is only claimed (implemented but not verified),
  and the rollback plan.
