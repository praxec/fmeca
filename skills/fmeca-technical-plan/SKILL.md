---
name: fmeca-technical-plan
description: Use when a design doc, architecture, interface contract or dependency choice needs a failure analysis before it is built, or when the user asks what could go wrong with how a system will be built. Opens a technical_plan analysis under the conops with fmeca-mcp (session.open, scoring.catalog, append, risk.next, readiness.assess) covering integration, data integrity, scaling, external dependencies and security, with design choices, guards and fallbacks as mitigations verified by ADRs and pull requests, then hands off to fmeca-project-plan and fmeca-implementation.
---

# fmeca-technical-plan: how the technology will be used

Calling the tools: see [Calling the tools](../fmeca/SKILL.md#calling-the-tools)
(registered MCP server or a host wrapper such as `junior tools call`).

Analyse the design before the code, at the `technical_plan` level: the components, the
interfaces between them, the data they hold and the services they depend on. The method and
the core loop are in [the fmeca reference](../fmeca/SKILL.md); level conventions are in
[levels.md](../fmeca/levels.md). If this skill and the server disagree, the server wins.

## 1. When to use

- A design doc, architecture proposal, ADR, interface contract or new dependency, before
  implementation starts.
- The user asks "what could go wrong with this design" or "is this architecture safe".
- A conops analysis exists and its failure modes have causes in the technology.
- Not for: a diff or module → use fmeca-implementation; sequencing → use fmeca-project-plan.

## 2. Inputs to read

- the design doc and any ADRs;
- the architecture: components, data stores, queues, trust boundaries;
- the interfaces: APIs, schemas, events, their versions and owners;
- the dependencies: external services, libraries, their limits (rate, size, latency);
- the non-functional requirements: load, latency, retention, recovery targets;
- the parent conops analysis: `state.get` on it, for the failure modes to refine.

## 3. Session

```json
{"session_id": "checkout-technical-plan", "title": "Checkout: technical plan",
 "level": "technical_plan", "aspect": "checkout",
 "scope_paths": ["docs/design/checkout.md", "docs/adr/0012-payments.md"],
 "parent": "checkout-conops"}
```

Choose `nasa8004_5x5` as `matrix_strategy` for safety- or mission-critical systems. Then run
the core loop: `scoring.catalog`, `append` failure modes, `risk.next`, `append` mitigations
in discipline order, until `risk.next` is null and the `readiness.assess` blockers are
worked.

## 4. What failure modes look like here

The `component` is a part of the design: a service, a store, an interface
(`checkout.payment_api`), a dependency (`dep.payment_provider`).

| Area | Typical failure modes | Domain |
|---|---|---|
| Integration | a contract mismatch, version skew, a retry that duplicates work, a partial failure across two services | `architecture` |
| Data integrity | a partial write, a lost update, a migration that drops data, a cache that serves stale truth | `runtime` |
| Scaling | a queue that grows without bound, a hot key, a fan-out that multiplies load | `runtime` |
| External dependencies | an outage, a rate limit, a slow response, a changed API | `architecture` |
| Security | a missing authorization check, a leaked secret, an unvalidated input crossing a trust boundary | `security` |

- Fill all three effects: `local_effect` on the component, `next_effect` on the operation
  above it, `effect` on the user or business. With a `parent` conops failure mode, write
  `next_effect` as that failure mode's cause.
- Set `single_point` when a component or dependency has no redundancy or fallback.
- Set `scope` (`localized`, `cross_cutting`, `structural`): it decides whether the fix is a
  `minor_fix`, a `restructure` or a `re_architecture`.

## 5. What counts as a mitigation, and what evidence verifies it

| Kind | At this level |
|---|---|
| `prevention` | a design choice that removes the failure: idempotency keys, transactions, a schema with validation, removing the dependency |
| `detection` | a guard that catches it: contract tests, health checks, reconciliation, alerts on an invariant |
| `fail_fast` | a fallback that bounds it: timeouts, circuit breakers, a degraded mode, bulkheads |

Evidence that verifies them (see [fmeca-resolve](../fmeca-resolve/SKILL.md)):
- the ADR or design doc that commits to the choice, as `file`, basis `design`;
- the `pull_request` that lands it, basis `design` or `inspection` (it was reviewed).

A design mitigation is `implemented` when its ADR or design PR merges. It is `verified` when
the code that carries it exists and is tested, which usually happens at the implementation
or verification level: record it there too.

To propose concrete work, call `actions.suggest` with `group_by: "tag"` (see
[fmeca-policy](../fmeca-policy/SKILL.md)); it only suggests.

## 6. Hand-off to the next level

- With a cpm-planner plan, or delivery risk on the critical path → hand off to
  [fmeca-project-plan](../fmeca-project-plan/SKILL.md) with `parent` this analysis.
- For each module or PR that carries a mitigation → hand off to
  [fmeca-implementation](../fmeca-implementation/SKILL.md) with `parent` this analysis. Pass
  the failure-mode ids the code must address: they are the implementation failure modes'
  `parent`s.
- Report to the user: the high and medium failure modes, the critical items, and the
  design decisions the mitigations need from them.
