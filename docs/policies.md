# Risk policies

fmeca computes criticality, residual risk and a readiness gate. A **risk
policy** re-colours those results into a project's own risk bands, sets a
sign-off rule, and suggests tagged actions — deterministically and at query
time. fmeca suggests; it never acts, and a policy is never written to the event
log, so one analysis can be read under several policies.

Policies are TOML: built-in **presets** (below) or project files in
`.fmeca/policies/<name>.toml`. A policy's identity is its `name` plus the
sha256 hash of its canonical form; every query reports both. A preset's hash
covers the preset **with the built-in action catalog merged in**, so a catalog
change changes every preset hash that includes it; a project policy's hash
covers only the file as written, and the catalog is merged at query time.

## The policy document

A policy has:

- **`bands`** — the risk ladder. Each band has a unique `id` and `rank`
  (higher is worse), the `dispositions` acceptable in it, an optional
  `min_confidence`, an optional `acceptance` role and `concurrence` roles, and
  an optional `review_interval` (`"<n>d"`). `no_action_needs_rationale` marks a
  band where leaving a failure mode without action needs a written rationale.
- **`matrices`** — at least one of `qualitative3x3` / `nasa8004_5x5`, mapping
  `(severity ordinal, probability ordinal)` to a band id, plus an optional
  `confidence` matrix (see `confidence-view`). Every S×P matrix declares the
  built-in layout it is `derived_from` and must be **monotone**: a cell's band
  rank never decreases as severity or probability increases.
- **`signoff`** — the band above which a residual fails (`max_residual_band`),
  whether an acceptance counts as a pass (`accepted_counts_as_pass`, default
  `true`; an acceptance counts only when the effective band's `dispositions`
  include `accept`, otherwise sign-off fails with `<fm>: accepted in band '<b>'
  which does not allow acceptance`), whether an overdue acceptance then fails
  (`accepted_requires_review_current`, default `false`), and whether the
  kernel's readiness **hygiene** blockers are added to the sign-off reasons as
  `kernel: <blocker>` (`include_kernel_gate`, default `true`; every built-in
  preset sets it to `true`). The hygiene blockers are `CLARIFY` (an open
  failure mode missing cause, effect or score), `DISCIPLINE` (weak mitigation
  order: fail-fast only) and `EMPTY` (no failure modes). The kernel's
  residual-zone blockers (`RESIDUAL: … still has Medium/High residual
  criticality`) are **not** added: which residuals are acceptable is the
  policy's own `max_residual_band`, so an unmitigated failure mode in
  `standard`'s `compensate` band passes even though the kernel calls it
  Medium. (The kernel reports blockers as strings; the stable `RESIDUAL:`
  prefix is the rule that leaves them out.) Finally, whether an unscored
  failure mode fails sign-off on its own as `<fm>: unscored`
  (`unscored_fails`, default `true`). With `unscored_fails = false` that
  reason is not emitted: an **open** unscored failure mode still fails through
  the kernel gate's `CLARIFY` blocker, and an **accepted** unscored one
  passes, exactly as 0.1 `readiness.assess` does. Only `compat-0.1` sets it to
  `false`.
- **`actions`** — optional policy-specific actions, plus the shared built-in
  catalog when `include_builtin_actions = true`.
- **`evidence_mode`** — optional, `"standard"` (the default) or `"strong"`:
  how much evidence a claimed reduction needs (see the fallback rule below).
  No built-in preset sets it.

Actions carry a `precedence` (MIL-STD-882E §4.3.4, adapted):
`eliminate` → `prevent` → `detect` → `contain` → `procedure`. They are
returned in that order, with `applies_when` deciding whether they are
suggested and `required_when` deciding whether they are required. The order is
a presentation, **not enforcement**: nothing stops a team from adding a
detection before a prevention. What is enforced is `required_when`.

An action of precedence `prevent` / `detect` / `contain` is marked
`covered_by` an implemented or verified mitigation of kind `prevention` /
`detection` / `fail_fast`; `eliminate` and `procedure` are never auto-covered.
A required action with no `covered_by` fails sign-off. No mitigation kind
covers a required `eliminate` or `procedure` action, so it is satisfied —
reported as `covered_by: ["acceptance"]` — when the failure mode is
**accepted** with a non-empty rationale **and** its effective band's
`dispositions` include `accept`. Otherwise it stays in `required_uncovered`
until its `required_when` stops holding (for example, the failure mode is
mitigated below the band, or its evidence reaches the named confidence).

`applies_when` / `required_when` keys (all optional; every key present must
hold): `band_at_least`, `band_in`, `rank_at_least` (the effective band's rank
is at least N — band-agnostic, so the shared catalog can use it; an unscored
failure mode never matches), `level_in`, `domain_in`, `response_class_in`,
`single_point`, `confidence_below`, `confidence_at_least`, `disposition_in`,
`fallback`, `class_in`, `mitigation_kinds_missing` (no implemented or verified
mitigation of these kinds) and `mitigation_kinds_present` (at least one
implemented or verified mitigation of these kinds). For both mitigation-kind
keys a **planned** mitigation does not count — a plan is not coverage, the
same rule as `covered_by` — so a planned prevention does not lift
`prevention-before-detection`, and a planned fail-fast does not trigger
`flag-procedural-only`. A policy's own `rank_at_least` may not exceed its
highest band rank (`INVALID_POLICY: actions[i].…rank_at_least`).

## Confidence ladder

Confidence is derived from the evidence on the mitigations that achieve the
residual; the best item wins. A **`note` is C0 whatever its basis** — evidence
must be locatable.

| Level | Meaning | Evidence that reaches it |
|---|---|---|
| C0 | Declared only | no evidence, or only `note` items |
| C1 | Analysed | basis `design`; or kind `file` / `url` without a basis |
| C2 | Inspected | basis `inspection`; or kind `pull_request` / `commit` |
| C3 | Tested | basis `test`, or kind `test` |
| C4 | Proven in use | basis `operational_use` or `failure_history` |

A failure mode with no mitigations is C0. An unscored failure mode is C0 and,
under `unscored_fails` (every preset but `compat-0.1`), fails sign-off as
`unscored`.

## Fallback rule (MIL-STD-882E Table VI)

A residual counts only when its confidence reaches the **required
confidence**, which the policy's `evidence_mode` sets:

- **`standard`** (the default): the residual band's own `min_confidence`.
  Only when the residual band sets none and the claim is a reduction, the
  `min_confidence` of the **nearest gated band above it** — the lowest-ranked
  band above the residual, at or below the inherent band, that sets one — so
  a reduction to an ungated lowest band is never free while a band above it
  is gated. Under `standard`, `review` → `compensate` needs C1
  (`compensate`'s own) and `review` → `none` needs C1 (borrowed from
  `compensate`); under `safety-critical`, `high` → `low` needs C2 (borrowed
  from `medium`).
- **`strong`** (opt-in, `evidence_mode = "strong"`): the highest
  `min_confidence` over the residual band and **every band the claimed
  reduction leaves**. Claiming to leave a band needs that band's evidence, so
  a bigger claimed reduction never needs less evidence: `review` → `none`
  needs C2 under `standard`'s bands, and `high` → `low` needs C3 under
  `safety-critical`'s. Choose it in regulated or safety contexts, where a
  claim to have left the worst band should carry the worst band's evidence.

For regulated or safety use, set `evidence_mode = "strong"` in a project
policy: every built-in preset, `safety-critical` included, uses `standard`.

When the confidence is below the required confidence, the **inherent** band
is effective instead, and the result carries `fallback {from, to, reason}`;
the reason names the band whose `min_confidence` set the requirement. A
fallback only ever raises the band: low confidence never lowers a residual,
and a fallback is only reported when the inherent band ranks above the
residual band.

Under `confidence-view`, the fallback is computed with the `standard`
preset's bands, so `fallback {from, to}` name **standard's** bands (`none` /
`compensate` / `review`), not `confidence-view`'s own. It uses the evaluated
policy's own `evidence_mode`: a copy of `confidence-view` with
`evidence_mode = "strong"` gets the strong rule on standard's bands.

## One-shot analysis input

`actions.suggest` with `analysis`, and `fmeca-mcp suggest --analysis
<file.json>`, read an `analyze` envelope rather than a session. That input
carries no analysis level, no single-point flags, no dispositions (every
failure mode is open) and no resolution evidence (every mitigation is
**planned**). So every residual is **C0**, every claimed reduction below a band
with a `min_confidence` falls back to the inherent band, nothing is
`covered_by` a mitigation, and `level_in` / `single_point = true` /
`mitigation_kinds_present` never match. One-shot input suits **screening**
("what would this policy flag?"); sign-off is meant for **sessions**, where
mitigations are resolved with evidence and risks can be accepted.

## Presets

All presets cover both `qualitative3x3` and `nasa8004_5x5`. In the cell tables
below, rows are severity ordinals (least severe first) and columns are
probability ordinals (least probable first).

### compat-0.1

The default and the backwards-compatibility gate: its sign-off equals the
fmeca 0.1 `readiness.assess` verdict. Two bands:

| id | rank | dispositions | min confidence | sign-off |
|---|---|---|---|---|
| `none` | 0 | `close`, `accept` | — | max residual band |
| `mitigate` | 1 | `mitigate`, `watch`, `accept` | — | — |

`qualitative3x3` (Low → `none`, Medium/High → `mitigate`):

```text
none     none     mitigate
none     mitigate mitigate
mitigate mitigate mitigate
```

`nasa8004_5x5` (Low → `none`, Medium/High → `mitigate`):

```text
none     none     none     mitigate mitigate
none     none     mitigate mitigate mitigate
none     mitigate mitigate mitigate mitigate
mitigate mitigate mitigate mitigate mitigate
mitigate mitigate mitigate mitigate mitigate
```

`signoff.max_residual_band = "none"`, `accepted_counts_as_pass = true`,
`unscored_fails = false` (0.1 readiness fails an unscored failure mode only
while it is open, through `CLARIFY`; an accepted unscored one passes).

### prototype

Two bands beyond the kernel's gate: report the risk, or remediate.

| id | rank | dispositions | acceptance | review |
|---|---|---|---|---|
| `none` | 0 | `close`, `accept` | — | — |
| `report` | 1 | `watch`, `accept`, `mitigate` | — | — |
| `remediate` | 2 | `mitigate`, `research`, `accept` | `owner` | 365d |

`qualitative3x3` (Low → `none`, Medium → `report`, High → `remediate`):

```text
none   none     report
none   report   remediate
report remediate remediate
```

`nasa8004_5x5`:

```text
none   none   none     report    report
none   none   report   report    remediate
none   report report   remediate remediate
report report remediate remediate remediate
report remediate remediate remediate remediate
```

`signoff.max_residual_band = "report"`.

### standard

The default for production software (the maintainer's chart 1).

| id | rank | dispositions | min confidence | acceptance | review |
|---|---|---|---|---|---|
| `none` | 0 | `close`, `accept` | — | — | — |
| `compensate` | 1 | `mitigate`, `watch`, `accept` | C1 | `tech_lead` | 180d |
| `review` | 2 | `mitigate`, `research`, `elevate`, `accept` | C2 | `eng_manager` (+ `tech_lead`) | 90d |

`qualitative3x3`:

```text
none       none       compensate
none       compensate review
compensate review     review
```

`nasa8004_5x5`:

```text
none       none       none       compensate compensate
none       none       compensate compensate review
none       compensate compensate review     review
compensate compensate review     review     review
compensate review     review     review     review
```

`signoff.max_residual_band = "compensate"`, `accepted_requires_review_current
= true`.

Required-action tailoring: `design-review` is **required** when the effective
band is `review` (`required_when = { band_at_least = "review" }`). It is only
suggested — and so only required — at the `technical_plan` and
`implementation` levels (`applies_when` has `level_in`); an analysis with no
level, or at another level, gets no design review from `standard`.
`design-review` is an `eliminate` action, which no mitigation covers: an open
`review` failure mode at those levels keeps it uncovered, and a reasoned
acceptance (allowed in `review`) satisfies it (`covered_by: ["acceptance"]`).

### business-critical

The `standard` bands with tighter evidence and tailoring.

| id | rank | dispositions | min confidence | acceptance | review |
|---|---|---|---|---|---|
| `none` | 0 | `close`, `accept` | — | — | — |
| `compensate` | 1 | `mitigate`, `watch`, `accept` | C2 | `tech_lead` | 90d |
| `review` | 2 | `mitigate`, `research`, `elevate`, `accept` | C3 | `eng_manager` (+ `tech_lead`, `product_owner`) | 90d |

The cell tables are those of `standard`. `signoff.max_residual_band =
"compensate"`.

Required actions, at every analysis level, when the effective band is
`review`:

| Action | `required_when` | Covered by |
|---|---|---|
| `design-review` | `band_at_least = "review"` | a reasoned acceptance (`eliminate`); else mitigate below `review` |
| `prevention-before-detection` | `band_at_least = "review"`, `mitigation_kinds_missing = ["prevention"]` | holds until an implemented or verified prevention exists (a planned one does not lift it) |
| `mitigate-single-point` | `band_at_least = "review"`, `single_point = true` | an implemented or verified `prevention` mitigation |

So a single point of failure left in `review` — even an accepted one — fails
sign-off until an implemented prevention covers it.

### safety-critical

MIL-STD-882E risk assessment codes: `low` / `medium` / `serious` / `high`.

| id | rank | dispositions | min confidence | acceptance | review |
|---|---|---|---|---|---|
| `low` | 0 | `close`, `accept` | — | — | — |
| `medium` | 1 | `mitigate`, `watch`, `accept` | C2 | — | — |
| `serious` | 2 | `mitigate`, `research`, `elevate`, `accept` | C3 | `eng_manager` (+ `product_owner`) | 90d |
| `high` | 3 | `mitigate`, `elevate` (not acceptable) | C3 | — | 90d |

`nasa8004_5x5` is MIL-STD-882E Table III. Columns are probability ordinals
1..5 = E Improbable, D Remote, C Occasional, B Probable, A Frequent. Severity
IV..I (Negligible, Marginal, Critical, Catastrophic) map to rows 2..5; row 1
is one band below Negligible.

```text
row 1 (below IV)   low    low     low     low     low
row 2 (IV)         low    low     low     medium  medium
row 3 (III)        medium medium  medium  serious serious
row 4 (II)         medium medium  serious high    high
row 5 (I)          medium serious high    high    high
                   E      D       C       B       A
```

`qualitative3x3` is the analogue: the middle of the 5×5 (rows and columns
2..4). 3×3 severity Low / Medium / High stands for Negligible IV / Marginal
III / Critical II, and probability Low / Medium / High for Remote D /
Occasional C / Probable B:

```text
low    low     medium
medium medium  serious
medium serious high
```

`signoff.max_residual_band = "medium"`. `high` does not list `accept`, so an
accepted `high` risk fails sign-off (`accepted in band 'high' which does not
allow acceptance`).

Required-action tailoring, at every analysis level:

| Action | `required_when` | Covered by |
|---|---|---|
| `design-review` | `band_at_least = "serious"` | a reasoned acceptance in `serious` (`eliminate`; `high` does not allow `accept`); else mitigate below `serious` |
| `prevention-before-detection` | `band_at_least = "serious"`, `mitigation_kinds_missing = ["prevention"]` | holds until an implemented or verified prevention exists (a planned one does not lift it) |
| `test-and-inspection-evidence` | `band_at_least = "serious"`, `confidence_below = "C3"` | a reasoned acceptance in `serious` (`procedure`); else holds until the residual's evidence reaches C3 (tested) |

`test-and-inspection-evidence` checks **test-level evidence only**: it is
required while the residual's confidence is below C3, and the confidence
ladder takes the best single evidence item, so one `test` item lifts it with
no inspection evidence. It does not check that test **and** inspection
evidence are both attached, as spec §6 put it — a deliberate deviation, since
the ladder ranks evidence rather than recording which methods were used. The
id is kept for stability; its text asks for test-level evidence and,
where it applies, inspection evidence too.

`flag-procedural-only` is suggested (not required) on `serious` and `high`
when the implemented or verified mitigations include a `fail_fast` and no
`prevention` or `detection`
(`mitigation_kinds_present = ["fail_fast"]`, `mitigation_kinds_missing =
["prevention", "detection"]`). The design order of precedence is presented by
action ordering only; it is not enforced beyond the required actions above.

### confidence-view

The maintainer's chart 2: the effective band is read from a matrix of the
**standard** preset's effective band rank (rows) and the residual confidence
(columns C4…C0).

| id | rank | dispositions | acceptance | review |
|---|---|---|---|---|
| `none` | 0 | `close`, `accept` | — | — |
| `report` | 1 | `watch`, `accept`, `mitigate` | — | — |
| `remediate` | 2 | `mitigate`, `research`, `accept` | `tech_lead` | 180d |

`[matrices.confidence]` (`derived_from = "standard-bands"`; rows `standard`
rank 0/1/2, columns C4 C3 C2 C1 C0):

```text
                C4     C3        C2        C1        C0
none            none   none      none      report    report
compensate      none   report    report    remediate remediate
review          report remediate remediate remediate remediate
```

The cells are the maintainer's chart 2. The chart's Confidence scale
Absolute / High / Moderate / Slight / None maps to C4 / C3 / C2 / C1 / C0, and
its Risk columns 1 / 3 / 5 map to `standard`'s `none` / `compensate` /
`review`.

The row is `standard`'s **effective** band, which already includes standard's
own fallback (a low-confidence residual falls back to its inherent band). Low
confidence therefore counts on both axes: once in choosing the row, and again
in choosing the column. Any `fallback {from, to}` reported under
`confidence-view` names standard's bands.

The S×P matrices are the `standard` layout mapped to this preset's bands
(`none` → `none`, `compensate` → `report`, `review` → `remediate`); they give
the inherent and residual bands. `signoff.max_residual_band = "report"`.

## Built-in action catalog

`include_builtin_actions = true` merges the shared catalog in
`crates/fmeca-policy/presets/actions.toml` (29 actions over the tags `design`,
`review`, `test`, `ops`, `process`, `docs` and `security`). A preset's own
action with the same id replaces the built-in one. Catalog conditions are
band-agnostic — every preset has its own band vocabulary — so the catalog
gates on `rank_at_least` (rank 0 is every preset's lowest band):

- low-cost hygiene is not gated on rank: `secure-defaults` (security domain)
  and `adr-for-accepted-risk` (accepted risks);
- every other action needs `rank_at_least = 1`, so a failure mode in the
  lowest band gets few or no suggestions;
- expensive actions need `rank_at_least = 2`, and a domain or single-point
  condition where it fits: `threat-model` and `penetration-test` (security),
  `chaos-experiment` (runtime single point), `load-test`, `slo-error-budget`,
  `backup-restore-test`, `documented-runbook`, `simplify-design`,
  `design-review`.

## Sources

This design follows mature practice without claiming compliance with any
standard.

| Practice | Source | Design consequence |
|---|---|---|
| Matrices and their interpretation are defined per project in its Risk Management Plan, not fixed by the standard. | NPR 8000.4C §3.2.2(10) Note 1 | Policies are project data (`.fmeca/policies/`), versioned by content hash. |
| Fixed disposition vocabulary, exactly one per risk: **accept, mitigate, watch, research, close, elevate**; mitigate/watch/research need an owner; accept needs rationale, assumptions and a review interval; watch needs triggers. | NPR 8000.4C §3.4.2(i–j), §3.2.2(12) | Bands declare which dispositions are acceptable; accepted risks get a review interval and a "review due" check. |
| Tailoring by class changes required process rigour, not matrix thresholds: payload classes A–D (risk tolerance), software classes A–E with a requirements mapping matrix. | NPR 8705.4A §3.1–3.2, App. D; NPR 7150.2D §2.1.6, §2.2.1, App. C | Presets keep a standard matrix and vary which actions are required vs suggested, minimum confidence and acceptance roles. |
| A standard matrix must be used unless a tailored one is formally approved and derived from the standard tables. | MIL-STD-882E §4.3.3(d) | Custom matrices declare `derived_from` a built-in matrix; `policy validate` checks it. |
| Risk Assessment Codes (High / Serious / Medium / Low), with acceptance authority rising with risk and user-representative concurrence for Serious/High. | MIL-STD-882E Table III, §4.3.7 | The `safety-critical` preset uses these bands; bands carry an `acceptance` role and optional `concurrence` roles. |
| Design order of precedence: eliminate by design → reduce by design → engineered features → warning devices → procedures/training. | MIL-STD-882E §4.3.4 | Actions carry a `precedence` and are returned in that order. |
| Missing or incomplete verification evidence: the risk contribution is documented at the worst level until the work is done or formally accepted. | MIL-STD-882E Table VI | Confidence below a band's minimum makes the failure mode fall back to its inherent band. |
| Verification by analysis, test, demonstration or inspection. | MIL-STD-882E §4.3.6; NASA TADI | The confidence ladder ranks evidence; the ordering itself is fmeca's convention. |
| Action Priority H/M/L replaces RPN; for H the team must act or document why current controls are acceptable. | AIAG-VDA FMEA Handbook 2019 (+ errata) [secondary] | Bands may set `no_action_needs_rationale`. |
| Acceptability criteria (acceptable / ALARP / unacceptable) are set by the organisation in its plan. | ISO 14971:2019 cl. 4.2 [secondary] | Criteria live in the policy, not the code. |
| Re-evaluate when circumstances change, when new failure data shows risk higher/lower than recognised, and at a defined review interval. | NPR 8000.4C §1.2.2.4, §3.2.2(12); MIL-STD-882E §4.3.7 | 0.2 implements the review-interval trigger (`as_of` vs acceptance time). |

Secondary sources where standards are paywalled: AIAG-VDA 2019, ISO 26262,
DO-178C, ISO 14971, IEC 61508. No standard publishes a confidence × risk
matrix; `confidence-view` is fmeca's own.
