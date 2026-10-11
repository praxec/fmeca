# Levels and links

The detail behind [the fmeca reference](SKILL.md): which level to analyse, what to read, which
parent to link, and what evidence closes a mitigation at that level.

## The levels

| Level | Question | Skill | Reads | Parent |
|---|---|---|---|---|
| `concept` | Is this worth building, and who can it hurt? | [fmeca-concept](../fmeca-concept/SKILL.md) | the idea or brief, user research | none |
| `conops` | How will the business run it? | [fmeca-concept](../fmeca-concept/SKILL.md) | operating scenarios, actors, business workflows | the `concept` |
| `technical_plan` | How will we build it? | [fmeca-technical-plan](../fmeca-technical-plan/SKILL.md) | design doc, architecture, interfaces, dependencies | the `conops` |
| `project_plan` | How will we deliver it? | [fmeca-project-plan](../fmeca-project-plan/SKILL.md) | the cpm-planner plan, deliverables, sequencing | the `technical_plan` |
| `implementation` | What can fail in the code? | [fmeca-implementation](../fmeca-implementation/SKILL.md) | the diff, module or PR | the `technical_plan` (or the `project_plan`) |
| `verification` | What did we prove? | [fmeca-verification](../fmeca-verification/SKILL.md) | test reports, CI, rollout and monitoring plans | the `implementation` |

Start at the highest level you have material for. A bug fix in one module may need only an
`implementation` analysis; a new product starts at `concept`. When a parent analysis does
not exist and you have no material for it, open the child without a `parent` and say so.

## Typical failure modes and mitigations by level

| Level | Typical failure modes (`domain`) | Mitigations | Evidence that verifies them (`kind`, `basis`) |
|---|---|---|---|
| `concept` | no real value, harm to users or bystanders, misuse, legal or ethical exposure (`ux`, `security`) | scope or positioning changes | a decision record: `url` or `file` (a `note` may explain it), basis `design` |
| `conops` | workflow breaks, operator error, hand-offs between people or teams, missed SLAs (`delivery`, `ux`) | process controls, runbooks, checklists | the runbook as `file`, basis `operational_use` |
| `technical_plan` | integration faults, data integrity, scaling limits, external dependencies (`architecture`, `runtime`, `security`) | design choices, guards, fallbacks | the ADR as `file`, or the `pull_request` that lands it, basis `design` |
| `project_plan` | critical-path slips, two deliverables editing one file, missing prerequisites (`delivery`) | re-sequencing, buffers, splits | `url` to the merged plan revision, basis `design` |
| `implementation` | code-level faults, error handling, concurrency, input validation (`runtime`, `security`) | code guards, validation, tests | `commit`, `pull_request`, `test`, basis `test` or `inspection` |
| `verification` | untested paths, rollout and rollback, observability gaps (`runtime`, `delivery`) | tests, alerts, staged rollout | `test`, `url` to the CI run, `file`, basis `test` or `operational_use` |

## Session ids

`<aspect>-<level>`, with `_` in the level written as `-`. One aspect, one chain:

```text
checkout-concept
checkout-conops            parent: checkout-concept
checkout-technical-plan    parent: checkout-conops
checkout-project-plan      parent: checkout-technical-plan
checkout-implementation    parent: checkout-technical-plan
checkout-verification      parent: checkout-implementation
```

When one aspect has several analyses at a level (two services, two PRs), make the aspect
narrower (`checkout-payments-implementation`) rather than adding a suffix.

## Links

**Session parent.** `session.open.parent` names the analysis one level up. It must exist when
you open the child, and it is fixed once opened. `session.list` with `parent` finds the
children of an analysis.

**Failure-mode parent.** A failure mode that refines a higher-level one carries
`"parent": {"session_id": "<parent analysis>", "failure_mode_id": "<id>"}`. The parent must
exist when you append. Write the child's `next_effect` as the parent's cause: the child is
one way the parent's cause comes about.

```json
{
  "id": "fm-retry-double-charge",
  "cause": "the payment client retries a timed-out charge without an idempotency key",
  "local_effect": "a second charge request reaches the provider",
  "next_effect": "retried charges are not deduplicated (the cause of checkout-technical-plan/fm-duplicate-payment)",
  "effect": "the customer is charged twice and loses trust",
  "parent": {"session_id": "checkout-technical-plan", "failure_mode_id": "fm-duplicate-payment"}
}
```

**Refs.** `refs` anchor a failure mode outside fmeca. They are free text and not checked, so
use exactly these forms:

| Ref | Points at |
|---|---|
| `cpm:<plan_id>/<deliverable_id>` | a cpm-planner deliverable |
| `path:<file>` | a file in the repo, relative to the root |
| `issue:<url>` | an issue |
| `pr:<url>` | a pull request |
| `commit:<sha>` | a commit |

## Evidence

Evidence items go on `set_mitigation_status` and `set_failure_mode_disposition`:
`{"kind": ..., "reference": ..., "basis": ...}`.

- `kind` says where it is: `commit`, `pull_request`, `test`, `file`, `url`, `note`.
- `basis` says why it counts (NASA's retention-rationale categories): `design`, `test`,
  `inspection`, `failure_history`, `operational_use`.
- A `note` is never enough on its own to verify a mitigation or exclude a failure mode.

## The resolution lifecycle

Every mitigation starts `planned`. It moves to `implemented` when the change merges, to
`verified` when a test or check proves it, or to `abandoned` (with a note) when it is
dropped; abandoned mitigations stop reducing residual risk. Every change is kept in its
`history`. A failure mode is `open` until it is `accepted` (residual risk kept on purpose,
with a human approver) or `excluded` (not credible, with evidence). See
[fmeca-resolve](../fmeca-resolve/SKILL.md).
