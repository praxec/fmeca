---
name: fmeca-implementation
description: Use when code needs a failure analysis, before or after it is written, for a diff, module or pull request, or when the user asks what could go wrong in this code. Opens an implementation analysis under the technical plan (or project plan) with fmeca-mcp (session.open, scoring.catalog, append, risk.next, readiness.assess) with path refs on each failure mode, covering code-level faults, error handling, concurrency and input validation, with code guards, validation and tests as mitigations verified by commits, pull requests and tests, then hands off to fmeca-verification.
---

# fmeca-implementation: what can fail in the code

Analyse a diff, a module or a pull request at the `implementation` level: where the code can
fail, what that does to the design above it, and which guard or test stops it. The method and
the core loop are in [the fmeca reference](../fmeca/SKILL.md); level conventions are in
[levels.md](../fmeca/levels.md). If this skill and the server disagree, the server wins.

## 1. When to use

- A pull request or diff is ready for review, or a module is about to be changed.
- The technical plan names a mitigation that this code carries.
- The user asks "what could break here", "is this change safe" or "review this for
  failure modes".
- Not for: the design → use fmeca-technical-plan; test and rollout evidence → use
  fmeca-verification.

## 2. Inputs to read

- the diff (`git diff <base>...HEAD`) or the module, and the tests next to it;
- the callers of what changed, and what it calls: errors, retries, timeouts, locks;
- the inputs that cross a trust boundary;
- the parent analysis (`state.get`): the technical-plan or project-plan failure modes this
  code must address. Each becomes the `parent` of the failure modes you add here.

## 3. Session

```json
{"session_id": "checkout-implementation", "title": "Checkout: payment client (PR 142)",
 "level": "implementation", "aspect": "checkout",
 "scope_paths": ["src/payment/client.rs", "src/payment/retry.rs"],
 "parent": "checkout-technical-plan"}
```

Then run the core loop: `scoring.catalog`, `append` failure modes, `risk.next`, `append`
mitigations in discipline order, until `risk.next` is null and the `readiness.assess`
blockers are worked.

## 4. What failure modes look like here

The `component` is the module or type (`checkout.payment_client`). Every failure mode has a
`path:` ref to the file, and a `pr:` or `commit:` ref when there is one:

```json
{"variant": "add_failure_mode", "session_id": "checkout-implementation", "failure_mode": {
  "id": "fm-retry-double-charge", "session_id": "checkout-implementation",
  "component": {"id": "checkout.payment_client"},
  "description": "A timed-out charge is retried and captured twice",
  "cause": "retry() resends the charge without an idempotency key",
  "local_effect": "a second charge request reaches the provider",
  "next_effect": "retried charges are not deduplicated",
  "effect": "the customer is charged twice",
  "domain": "runtime", "scope": "localized", "single_point": true,
  "severity_observations": ["data_loss"],
  "probability_observations": ["load_or_concurrency_dependent"],
  "parent": {"session_id": "checkout-technical-plan", "failure_mode_id": "fm-duplicate-payment"},
  "refs": ["path:src/payment/retry.rs"], "source": {"turn_id": "review-pr-142"}}}
```

| Area | Typical failure modes | Domain |
|---|---|---|
| Faults | an unchecked `None` or index, an overflow, a wrong unit, an off-by-one | `runtime` |
| Error handling | a swallowed error, a retry without a bound or without idempotency, a partial cleanup | `runtime` |
| Concurrency | a race, a deadlock, a lost update, a non-atomic check-then-act | `runtime` |
| Input | unvalidated input across a trust boundary, injection, a missing authorization check | `security` |

- `next_effect` is how the fault shows up in the design: write it as the parent
  failure mode's cause.
- Set `single_point` when nothing else catches the fault: no retry, no fallback, no other
  check downstream.

## 5. What counts as a mitigation, and what evidence verifies it

| Kind | At this level |
|---|---|
| `prevention` | a type or constraint that makes the fault impossible, input validation, an idempotency key, a lock |
| `detection` | an assertion, a test that fails on the fault, a log or metric on the error path |
| `fail_fast` | a bounded retry, a timeout, returning an error instead of continuing |

Evidence that verifies them (see [fmeca-resolve](../fmeca-resolve/SKILL.md)):
- the `commit` or `pull_request` that adds the guard: `implemented` when it merges;
- the `test` that proves it, basis `test`: `verified` when it passes in CI. A review of the
  guard without a test is basis `inspection`.

## 6. Hand-off to the next level

- Hand off to [fmeca-verification](../fmeca-verification/SKILL.md) with `parent` this
  analysis when the change ships: it checks the tests, the rollout and the monitoring.
- As each guard merges and each test lands, move its mitigation with
  [fmeca-resolve](../fmeca-resolve/SKILL.md).
- Report to the user: the high and medium failure modes with their file refs, the critical
  items, and the guards and tests still to write.
