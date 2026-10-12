---
name: fmeca-signoff
description: "Use when an fmeca analysis is finished and someone needs a sign-off summary: before a release, a merge, a design approval or a phase gate, or when the user asks whether an analysis is ready. Runs readiness.assess and report.export, applies the right gate (verified.ready at the verification level, ready elsewhere), lists critical_items and accepted_risks with their approvers, shows Jev's readiness probability beside both gates and never instead of them, and attaches the .fmeca/<session_id>.jsonl log and the export."
---

# fmeca-signoff: the gate and the summary

Sign-off turns an analysis into a decision a human can make. You assemble the facts; the
user signs. The gates are defined in [the fmeca reference](../fmeca/SKILL.md). If this
skill and the server disagree, the server wins.

## When to use

- The level skill's loop is done: `risk.next` is null (it skips excluded and accepted
  failure modes) and the blockers have been worked.
- A release, merge, design approval or phase gate needs the analysis.
- The user asks "are we ready" or "can I sign this off".

## 1. Assess

Run `readiness.assess {"session_id": "<id>"}`. Read:
- `ready` and `blockers`: the gate over every active mitigation;
- `verified.ready` and `verified.blockers`: the same gate over `verified` mitigations only;
- `critical_items`, `accepted_risks`, `excluded` and `resolution`.

**The gate that applies:**
- at the `verification` level, sign-off requires `verified.ready`;
- at every other level, `ready` is the gate, and `verified.ready` is reported beside it;
- if the gate is false, do not present a sign-off. List the blockers and route them:
  `CLARIFY` and `RESIDUAL` to the level skill, `DISCIPLINE` to a stronger mitigation,
  unproven mitigations to [fmeca-resolve](../fmeca-resolve/SKILL.md).

For a release, also check the analyses above it: `session.list` with the same `aspect`
shows `ready` and `ready_verified` for every level at once.

## 1b. The project's policy (optional)

If the project has a risk policy (a `.fmeca/policies/` folder, or the user names a preset),
run `actions.suggest {"session_id": "<id>", "policy": "<name>"}` and show its `signoff.pass`,
`signoff.reasons` and `rollups.required_uncovered` **beside** the kernel gates, never instead
of them. `signoff.pass` is the policy verdict, not the kernel gate: with
`include_kernel_gate` (true in every preset) kernel blockers appear in the reasons as
`kernel: ...`. Report both, and never present the policy pass as the gate. Unscored failure
modes fail policy sign-off. It is read-only. Name the policy and its hash. See
[fmeca-policy](../fmeca-policy/SKILL.md). Without a policy, skip this.

## 2. Export

Run `report.export {"session_id": "<id>"}`. It returns a row per failure mode (with its
effects, `single_point`, `disposition`, criticality, residual, standing and mitigations with
their status and evidence), `residual_buckets`, `accepted_risks`, `critical_items` and the
`blockers`. Attach it to the sign-off (a PR comment, a release record) as JSON or a table.

## 3. Jev's view (optional)

If the user wants it and the analysis may be shared, run
[fmeca-vet](../fmeca-vet/SKILL.md) with `checks` `["readiness"]`. Present its readiness
probability **beside** `ready` and `verified.ready`, never instead of them. A low
probability with a true gate is a reason to look again, not a failed gate; a high one
with a false gate changes nothing. If it is unavailable, say so.

## 4. The sign-off summary

```text
FMECA sign-off: <title> (<session_id>, level <level>)
Gate:           ready=<true|false>  verified.ready=<true|false>   (applies: <which>)
Policy:         <name> <hash>: signoff=<pass|fail>, uncovered required actions <n>   | none
Jev readiness:  <p> against 0.5 (advisory)   | not run | unavailable: <reason>
Failure modes:  <n> (acceptable <a>, under-mitigated <u>, unmitigated <m>), excluded <e>
Mitigations:    planned <p>, implemented <i>, verified <v>, abandoned <x>
Critical items: <fm id: description, residual, mitigation status>   (or "none")
Accepted risks: <fm id: rationale, approved by <approver>>        (or "none")
Blockers:       <verbatim, or "none">
Log:            .fmeca/<session_id>.jsonl
Export:         <where the report.export output is attached>
```

- List **every** critical item and **every** accepted risk explicitly, each accepted risk
  with its approver from the failure mode's `disposition_history` (`state.get`). Never
  summarise them as a count.
- The log path is `session.list.state_dir` joined with `<session_id>.jsonl`; run
  `fmeca-mcp check` and confirm it prints `ok <session_id>` before you attach it.
- Make sure the `.fmeca/` changes are committed with the work they describe.
- End by asking the user to sign off. Do not record a sign-off on their behalf.
