---
name: fmeca-vet
description: "Use when an fmeca analysis would benefit from a second opinion before sign-off or review, or when the user asks whether the observations, mitigations or coverage of an analysis are sound. Runs the advisory vet tool (Jev on OpenRouter, needs OPENROUTER_API_KEY) on a session or a one-shot analysis, reads each finding's probability against its threshold, and acts: re-check flagged observations and rescore if warranted, reclassify or strengthen mitigations, decide whether coverage candidates deserve a failure mode. Continues without it on vet_unavailable. Never treats a probability as a verdict."
---

# fmeca-vet: a calibrated second opinion

Calling the tools: see [Calling the tools](../fmeca/SKILL.md#calling-the-tools)
(registered MCP server or a host wrapper such as `junior tools call`).

`vet` asks Jev, a calibrated judgment model, about an analysis and returns probabilities
beside the deterministic gate. It never writes and never changes a score, an observation
or a gate: anything that changes goes through `append`, after you have checked the evidence.
The method is in [the fmeca reference](../fmeca/SKILL.md). If this skill and the server
disagree, the server wins.

## When to use

- Before [fmeca-signoff](../fmeca-signoff/SKILL.md), or before asking a human to review an
  analysis.
- The user asks "is this analysis any good", "did we miss anything", or "are these scores
  right".
- Not when the analysis must not leave the machine: `vet` sends the failure modes,
  mitigations and the session's level and aspect to OpenRouter. Ask the user if unsure.

## 1. Run it

```json
{"session_id": "checkout-implementation"}
```

- Or pass `analysis` (the `analyze` arguments) instead of `session_id`, never both.
- `vet.checks` picks checks (default all four): `observations`, `mitigations`, `coverage`,
  `readiness`. Readiness and coverage are always asked first.
- `vet.max_questions` caps the questions sent (default 40, at most 200). The rest are listed
  in `skipped` by hash. If much was skipped, run again with fewer checks or a higher cap,
  and tell the user the cost.
- Report `usage.cost` (USD) to the user.

## 2. Read the findings

Each finding has `check`, `target`, `probability`, `threshold` and `verdict` (`flag` or
`ok`; for readiness, `concern` or `ok`). A `question_hash` identifies the question, so a
re-run on the same state asks the same questions. Read the probability against the
threshold, not just the verdict: 0.31 against a 0.3 threshold is a coin toss.

| Finding | Target | Flagged when | What to do |
|---|---|---|---|
| a claimed observation (`claimed: true`) | `<fm>/<observation>` | `probability` below 0.3 | re-read the evidence for that observation. If it really does not apply, `append` a `rescore` ([shape](../fmeca/SKILL.md#2-observations-not-scores)) that resends **both** axes in full, without that observation, and a `source`. If it does, keep it and say why |
| an unclaimed observation (`claimed: false`) | `<fm>/<observation>` | `probability` above 0.7: the next level up may apply | consider it. If the evidence supports it, `rescore` with both axes in full (the reference's example); otherwise leave it |
| a mitigation that may not address its cause | `<fm>/<mitigation>` | below 0.5 | strengthen it, add another mitigation, or make its residual observations honest |
| a class mismatch (`declared` vs `choice`) | `<fm>/<mitigation>#class` | another kind is at least 0.6 likely | if Jev is right, the kind is wrong: add a mitigation of the right kind (the old one cannot be edited; abandon it with [fmeca-resolve](../fmeca-resolve/SKILL.md)), or strengthen it until the declared kind is true |
| a coverage candidate | `coverage/<class>` | at least 0.6 that a class is uncovered | decide whether a real failure mode of that class exists here. If it does, `append` it with its own cause and evidence. Jev never invents failure modes, and neither should you on its say-so |
| readiness | `readiness` | below 0.5 (`concern`) | read beside `readiness_gate`; see [fmeca-signoff](../fmeca-signoff/SKILL.md) |

The coverage classes are `data_integrity`, `concurrency`, `external_dependency`, `security`,
`resource_exhaustion`, `configuration` and `operator_error`.

Excluded failure modes are not vetted.

## 3. When it is unavailable

`{"status": "vet_unavailable", "reason": ...}` means no second opinion this time: `no_key`
(no `OPENROUTER_API_KEY` in the server's environment), `timeout`, `http_4xx`, `http_5xx`,
`network`, `bad_response` or `bad_config`. It still returns `readiness_gate`. Continue
without it, and tell the user the analysis was not vetted and why. Never retry in a loop.

## 4. Report

Summarise for the user: the flags you acted on (and the `append` you made), the flags you
kept with your reason, the coverage candidates you added or rejected, and Jev's readiness
probability beside the deterministic gate. The gate decides; the probabilities inform.
