---
name: fmeca-project-plan
description: "Use when a delivery plan needs a failure analysis: the deliverables, their sequencing and the critical path of a cpm-planner plan (plan.* tools), or the user asks what could make delivery slip. Opens a project_plan analysis under the technical plan with fmeca-mcp, reads plan.status (critical_path, schedule float, milestones) through a Junior hop or the cpm MCP server, puts cpm:<plan_id>/<deliverable_id> refs on failure modes such as critical-path slips, file contention and missing prerequisites, and verifies re-sequencing, buffers and splits by the merged plan revision."
---

# fmeca-project-plan: how the work will be delivered

Calling the tools: see [Calling the tools](../fmeca/SKILL.md#calling-the-tools)
(registered MCP server or a host wrapper such as `junior tools call`).

Analyse the plan, not the product, at the `project_plan` level: which deliverables can slip,
collide or start without what they need. The method and the core loop are in [the fmeca
reference](../fmeca/SKILL.md); level conventions are in [levels.md](../fmeca/levels.md). If
this skill and the server disagree, the server wins.

## 1. When to use

- A cpm-planner plan exists (or is being written) for work a technical plan describes.
- The user asks "what could make this slip", "what is risky about this schedule", or
  "which deliverables could block each other".
- Not for: the design itself → use fmeca-technical-plan; authoring or changing the plan →
  use cpm-planner's cpm-plan and cpm-revise skills.

## 2. Inputs to read

Read the cpm-planner plan when one exists:
- the plan file, `.cpm-planner/plans/<name>/<variant>.json`: deliverables, their
  prerequisites (what each consumes), `owned_files` and estimates;
- `plan.status {"plan_id": "<plan_id>"}`: `critical_path`, `schedule` (each deliverable's
  `float`), `milestones` and `ready`. Find the `plan_id` with `plan.list`, or ask the user.

Call cpm-planner either through its MCP server, or through Junior's hop:

```bash
echo '{"tool": "plan.status", "arguments": {"plan_id": "<plan_id>"}}' > request.json
junior tools call <hop> cpm request.json
```

Also read the parent technical plan (`state.get`) for the failure modes whose mitigations
are deliverables here. Without a cpm plan, read the project plan the user has (milestones,
tickets, owners) and skip the `cpm:` refs.

## 3. Session

```json
{"session_id": "checkout-project-plan", "title": "Checkout: delivery plan",
 "level": "project_plan", "aspect": "checkout",
 "scope_paths": [".cpm-planner/plans/checkout/main.json"], "parent": "checkout-technical-plan"}
```

Then run the core loop: `scoring.catalog`, `append` failure modes, `risk.next`, `append`
mitigations in discipline order, until `risk.next` is null and the `readiness.assess`
blockers are worked.

## 4. What failure modes look like here

The `component` is the deliverable (`deliverable.payments-client`) and every failure mode
carries a ref to it: `"refs": ["cpm:<plan_id>/<deliverable_id>"]`.

| Typical failure mode | What to look for |
|---|---|
| a critical-path slip | a deliverable on `critical_path`, or with little `float`, with an uncertain estimate or an unproven dependency |
| file contention | two deliverables that write the same file without an edge between them |
| a missing prerequisite | a deliverable that consumes something no edge provides: a contract, a decision, provisioned infrastructure |
| a milestone without acceptance | a milestone whose acceptance no one owns |
| a single executor | work only one person or agent can do |

- The domain is `delivery`.
- Fill all three effects: `local_effect` on the deliverable, `next_effect` on the milestone
  or the deliverables that consume it, `effect` on the release. With a `parent` technical
  plan failure mode, write `next_effect` as that failure mode's cause.
- Set `single_point` when only one executor can do the work, or nothing can replace a
  deliverable on the critical path.
- Probability observations come from the schedule: `happens_in_normal_use` when the
  deliverable's pessimistic estimate minus its likely estimate exceeds its `float` (any
  slip within the estimate's range moves the milestone); `occasional` when that spread is
  within the float but uses more than half of it; `rare_edge_case` when it uses less.

## 5. What counts as a mitigation, and what evidence verifies it

| Kind | At this level |
|---|---|
| `prevention` | re-sequence: add the missing edge, order two deliverables that share a file, split a deliverable along a contract seam |
| `detection` | a milestone or check that shows the slip early: a contract deliverable accepted first, an earned-value snapshot cadence |
| `fail_fast` | a buffer, or a cut line: the scope dropped first if the milestone is at risk |

Make the change in the plan file with cpm-planner (its cpm-revise skill: lint, then sync).
The evidence that verifies it is a `url` to the merged plan revision (the commit or pull
request that changed the plan file), basis `design`. See
[fmeca-resolve](../fmeca-resolve/SKILL.md).

To propose concrete work, call `actions.suggest` with `group_by: "tag"` (see
[fmeca-policy](../fmeca-policy/SKILL.md)); it only suggests.

## 6. Hand-off to the next level

- Hand off to [fmeca-implementation](../fmeca-implementation/SKILL.md) for each deliverable
  whose code carries risk, with `parent` this analysis (or the technical plan, when the risk
  is in the design rather than the delivery).
- Report to the user: the deliverables at risk with their `cpm:` refs, the plan changes
  proposed, and any buffer or cut line they must approve.
