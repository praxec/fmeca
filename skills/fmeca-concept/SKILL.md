---
name: fmeca-concept
description: Use when a product idea, feature brief or business change needs a risk analysis before anything is designed, or when the user asks what could go wrong with an idea or with how the business will run it. Decides between a concept analysis (value, harm, misuse, legal and ethical risk) and a conops analysis (actors, workflows, hand-offs, SLAs) with the concept as parent, or runs both in sequence, using the fmeca-mcp tools session.list, session.open, scoring.catalog, append and risk.next, then hands off to fmeca-technical-plan.
---

# fmeca-concept: the idea, then how the business runs it

Analyse the thing before it is designed: first whether it is worth building and whom it can
hurt (`concept`), then how the business will operate it (`conops`). The method, the
observation rules and the core loop are in [the fmeca reference](../fmeca/SKILL.md); level
conventions are in [levels.md](../fmeca/levels.md). If this skill and the server ever
disagree, the server's behaviour wins.

## Which lens?

Read the input, then pick:
- **a product idea, feature or offer** (what it is, who it is for, why) → a `concept`
  analysis;
- **how the business will run it** (who does what, workflows, support, SLAs) → a `conops`
  analysis whose `parent` is the concept;
- **both** (a brief that covers the offer and its operation) → `concept` first, then
  `conops`. Do them in that order: conops failure modes often refine concept ones.

Run `session.list` with `aspect` first. If the concept analysis already exists, resume it or
go straight to conops. If the user only has operating material and no concept exists, open
the conops without a parent and say so.

## 1. When to use

- A new product, feature, market or business process, before a design exists.
- The user asks "what could go wrong with this idea", "who could this harm", or "can we
  operate this".
- Not for: architecture and interfaces → use fmeca-technical-plan; code → use
  fmeca-implementation.

## 2. Inputs to read

- `concept`: the idea or brief, the problem statement, user research, the intended users
  and the people it affects without using it, and any legal or policy constraints.
- `conops`: the operating scenarios, the actors (customers, operators, support, partners),
  the business workflows and their hand-offs, support hours and SLAs.

Ask the user for what is missing rather than inventing it. Put the documents you read in
`scope_paths`.

## 3. Session

```json
{"session_id": "checkout-concept", "title": "One-click checkout: concept", "level": "concept",
 "aspect": "checkout", "scope_paths": ["docs/briefs/one-click-checkout.md"]}
```

```json
{"session_id": "checkout-conops", "title": "One-click checkout: operations", "level": "conops",
 "aspect": "checkout", "scope_paths": ["docs/ops/checkout-runbook.md"], "parent": "checkout-concept"}
```

Then run the core loop from the reference: `scoring.catalog`, `append` failure modes,
`risk.next`, `append` mitigations in discipline order, until `risk.next` is null and the
`readiness.assess` blockers are worked.

## 4. What failure modes look like here

The `component` is where the failure starts: a capability (`capability.one_click_buy`), an
actor (`actor.support_agent`) or a workflow step (`workflow.refund`).

| Lens | Typical failure modes | Domains |
|---|---|---|
| `concept` | users get no real value; harm to users or bystanders (privacy, safety, money); misuse and abuse; legal, regulatory or ethical exposure; accessibility exclusion | `ux`, `security` |
| `conops` | a workflow breaks or stalls; operator error; a hand-off between people or teams drops work; an SLA is missed; nobody owns an exception case | `delivery`, `ux` |

- Fill all three effects. At `concept` the `local_effect` is on the person or the offer, the
  `next_effect` on the business, and `effect` the end result. At `conops`, with a `parent`
  concept failure mode, write `next_effect` as that failure mode's cause.
- Set `single_point` when one person, team or partner is the only way a step gets done.
- The scoring catalog is written in system terms. Read each criterion's `description` and
  choose the one that fits the effect: a privacy harm is `security_breach`, a broken
  promise to users that they can work around is `user_facing_degradation`. If nothing fits,
  take the nearest more severe one and say why in the `description`.
- `requires_misuse` is the probability for abuse cases; do not use it to wave a harm away.

## 5. What counts as a mitigation, and what evidence verifies it

| Lens | Prevention | Detection | Fail fast |
|---|---|---|---|
| `concept` | change the scope or positioning; drop the harmful feature; restrict who can use it | a pilot or user test that shows whether the value is real | kill criteria and a staged launch |
| `conops` | a process control: a checklist, a two-person rule, an owner for each exception | monitoring or a queue that shows stalled work | a runbook that bounds the damage when it happens |

Evidence that verifies them (see [fmeca-resolve](../fmeca-resolve/SKILL.md)):
- `concept`: the decision record that changed the scope, as `url` (or `file`), basis
  `design`. A `note` can explain it but cannot verify it.
- `conops`: the runbook or process document as `file`, basis `operational_use` once it has
  been exercised (a drill or a real incident).

To propose concrete work, call `actions.suggest` with `group_by: "tag"` (see
[fmeca-policy](../fmeca-policy/SKILL.md)); it only suggests.

## 6. Hand-off to the next level

- After `concept`, open the `conops` with `parent` the concept.
- After `conops`, hand off to [fmeca-technical-plan](../fmeca-technical-plan/SKILL.md) with
  `parent` the conops. Tell it which conops failure modes have their cause in the
  technology: those are the parents of technical-plan failure modes.
- Report to the user: the high and medium failure modes, the mitigations proposed, and
  whatever they must decide (a scope change, an accepted risk). Scope changes are theirs.
