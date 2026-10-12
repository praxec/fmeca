---
name: fmeca-resolve
description: "Use when a mitigation in an fmeca analysis has been built, proven or dropped (a pull request merged, a test landed, a runbook was exercised), when the user wants to accept a residual risk, or when a failure mode turns out not to be credible. Moves mitigations through planned, implemented, verified and abandoned with append set_mitigation_status and evidence (kind and basis), records accepted or excluded failure modes with append set_failure_mode_disposition (a human approver for acceptance, non-note evidence for exclusion; never delete a failure mode), and ends with readiness.assess: verified.blockers, accepted_risks and critical_items."
---

# fmeca-resolve: from planned to proven

An analysis says what *should* be done. Resolution records what *was* done and how we know.
The lifecycle and the evidence rules are in [the fmeca reference](../fmeca/SKILL.md) and
[levels.md](../fmeca/levels.md). If this skill and the server disagree, the server wins.

## When to use

- A pull request that carries a mitigation merged → `implemented`.
- A test, check, drill or review proved it → `verified`.
- A mitigation was dropped or replaced → `abandoned`.
- The user decides to keep a residual risk → disposition `accepted`.
- A failure mode cannot happen after all → disposition `excluded`.

Find the ids first: `session.list` shows each analysis's `resolution` counts, and
`state.get` lists every failure mode with its mitigations, their `status` and `history`.

## 1. Move a mitigation

```json
{"variant": "set_mitigation_status", "session_id": "checkout-implementation",
 "mitigation_id": "m-idempotency-key", "status": "implemented",
 "evidence": [{"kind": "pull_request", "reference": "https://github.com/acme/shop/pull/142",
               "basis": "inspection"}]}
```

```json
{"variant": "set_mitigation_status", "session_id": "checkout-implementation",
 "mitigation_id": "m-idempotency-key", "status": "verified",
 "evidence": [{"kind": "test", "reference": "tests/payment_retry.rs::retry_reuses_idempotency_key",
               "basis": "test"},
              {"kind": "url", "reference": "https://ci.example/runs/9912", "basis": "test"}]}
```

| Status | When | Evidence |
|---|---|---|
| `planned` | every mitigation starts here | none |
| `implemented` | the change that carries it merged | the `commit` or `pull_request` |
| `verified` | a test or check proves it works | at least one item whose `kind` is not `note`: `test`, `url` (CI run), `file` (runbook, ADR) |
| `abandoned` | it was dropped or replaced | the top-level `note` saying why |

- Give every evidence item a `basis`: `design`, `test`, `inspection`, `failure_history` or
  `operational_use`. `kind` says where the evidence is; `basis` says why it counts.
- Evidence accumulates across changes; the server stamps each change's `at`.
- Verify only what the evidence proves. A merged PR is not a passing test. Do not mark
  `verified` on a test you have not seen pass.
- To abandon, explain in the top-level `note` field, not in an evidence item:

```json
{"variant": "set_mitigation_status", "session_id": "checkout-implementation",
 "mitigation_id": "m-request-timeout", "status": "abandoned",
 "note": "Replaced by m-provider-fallback: a timeout alone only bounds the wait"}
```

- An `abandoned` mitigation stops reducing residual risk, so the failure mode may become a
  blocker again. Run `risk.next` and add the replacement mitigation.
- Evidence found at one level often proves a mitigation at another (a verification test
  proves an implementation guard). Set the status in the session that owns the mitigation.

## 2. Accept a risk

Acceptance keeps a residual risk on purpose. It is a human decision:

1. Show the user the failure mode, its residual criticality and why it cannot reasonably be
   mitigated further.
2. **Ask who approves it.** Never name yourself, an agent or a tool as the approver, and
   never guess a name. If no one approves, leave the failure mode `open`.
3. Record it with their rationale and their name, exactly as given:

```json
{"variant": "set_failure_mode_disposition", "session_id": "checkout-technical-plan",
 "failure_mode_id": "fm-provider-outage", "disposition": "accepted",
 "rationale": "A two-provider setup costs more than a rare hour of lost checkout; status page and retry banner cover it",
 "approver": "Dana Smith (head of payments)"}
```

In the kernel, an accepted failure mode stops blocking both gates and is listed in
`accepted_risks`. A risk policy is stricter: see section 4.

## 3. Exclude a failure mode that is not credible

Use `excluded` when the failure cannot happen, never because it is inconvenient. It needs a
`rationale` and at least one evidence item that is not a `note`:

```json
{"variant": "set_failure_mode_disposition", "session_id": "checkout-implementation",
 "failure_mode_id": "fm-float-rounding", "disposition": "excluded",
 "rationale": "Amounts are integer cents end to end; no float arithmetic touches money",
 "evidence": [{"kind": "file", "reference": "src/payment/money.rs", "basis": "inspection"}]}
```

**Never delete a failure mode**; there is no way to, and a log edited by hand fails
`fmeca-mcp check`. Exclusion leaves it in the history, out of `risk.next` and the gates,
and listed in `excluded`. To undo an acceptance or exclusion, set the disposition back to
`open` with a rationale.

## 4. Use the policy's suggestions

Under a risk policy, `actions.suggest` gives each failure mode a `status` with a `suggested`
disposition and lists `rollups.review_overdue`, the acceptances past their review date. Use
them to decide what to raise with the user: a failure mode suggested `accept` still needs a
named approver, and an overdue acceptance needs a fresh decision. A `fallback` means the
mitigation's evidence is too weak for the band: add the test or inspection evidence (section
1), do not accept to make it go away. Acceptance, exclusion and status stay with the human.

**Acceptance is band-gated under a policy.** It counts as a pass only when the effective
band's `dispositions` include `accept`; otherwise policy sign-off fails with "accepted in
band ... which does not allow acceptance". The same rule decides whether a required
`eliminate` or `procedure` action is covered (`covered_by: ["acceptance"]`), and it also
needs a non-empty rationale. Check the band before you ask for an approver. See
[fmeca-policy](../fmeca-policy/SKILL.md) and [docs/policies.md](../../docs/policies.md).

## 5. Report

Run `readiness.assess` on every session you changed and report:
- `ready` and `verified.ready`, with `verified.blockers`: what is still only claimed;
- `accepted_risks`, each with its approver (from the failure mode's `disposition_history`
  in `state.get`);
- `critical_items`: single points with high raw criticality. Each needs a verified
  mitigation, or an explicit acceptance the user knows about.
