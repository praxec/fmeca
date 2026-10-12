# Risk policies

fmeca computes criticality, residual risk and a readiness gate. A **risk
policy** re-colours those results into a project's own risk bands, sets a
sign-off rule, and suggests tagged actions — deterministically and at query
time. fmeca suggests; it never acts, and a policy is never written to the event
log, so one analysis can be read under several policies.

Policies are TOML: built-in **presets** (below) or project files in
`.fmeca/policies/<name>.toml`. A policy's identity is its `name` plus the
sha256 hash of its canonical form; every query reports both.

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
  `true`), whether an overdue acceptance then fails
  (`accepted_requires_review_current`, default `false`), and whether the
  kernel's readiness blockers are added to the sign-off reasons
  (`include_kernel_gate`, default `true`; every built-in preset sets it to
  `true`).
- **`actions`** — optional policy-specific actions, plus the shared built-in
  catalog when `include_builtin_actions = true`.

Actions carry a `precedence` (MIL-STD-882E §4.3.4, adapted):
`eliminate` → `prevent` → `detect` → `contain` → `procedure`. They are
returned in that order, with `applies_when` deciding whether they are
suggested and `required_when` deciding whether they are required. An action of
precedence `prevent` / `detect` / `contain` is marked `covered_by` an active
mitigation of kind `prevention` / `detection` / `fail_fast`; `eliminate` and
`procedure` are never auto-covered. Required actions with no `covered_by` fail
sign-off.

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

A failure mode with no mitigations is C0. An unscored failure mode is C0 and
fails sign-off as `unscored`.

## Fallback rule (MIL-STD-882E Table VI)

A residual counts in its band only at or above that band's `min_confidence`.
When the confidence is below it, the **inherent** band is effective instead,
and the result carries `fallback {from, to, reason}`. A fallback only ever
raises the band: low confidence never lowers a residual, and a fallback is
only reported when the inherent band ranks above the residual band.

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

`signoff.max_residual_band = "none"`, `accepted_counts_as_pass = true`.

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
= true`. Required-action tailoring: `design-review` on `review` for the
`business-critical` and `safety-critical` classes.

### business-critical

The `standard` bands with tighter evidence and tailoring.

| id | rank | dispositions | min confidence | acceptance | review |
|---|---|---|---|---|---|
| `none` | 0 | `close`, `accept` | — | — | — |
| `compensate` | 1 | `mitigate`, `watch`, `accept` | C2 | `tech_lead` | 90d |
| `review` | 2 | `mitigate`, `research`, `elevate`, `accept` | C3 | `eng_manager` (+ `tech_lead`, `product_owner`) | 90d |

The cell tables are those of `standard`. Additional required actions: on a
`review` risk, an engineered **prevention** must exist before detection
(`require-prevention`), and a single point of failure in `review` must be
mitigated (`mitigate-single-point`). `signoff.max_residual_band = "compensate"`.

### safety-critical

MIL-STD-882E risk assessment codes: `low` / `medium` / `serious` / `high`.

| id | rank | dispositions | min confidence | acceptance | review |
|---|---|---|---|---|---|
| `low` | 0 | `close`, `accept` | — | — | — |
| `medium` | 1 | `mitigate`, `watch`, `accept` | C2 | — | — |
| `serious` | 2 | `mitigate`, `research`, `elevate`, `accept` | C3 | `eng_manager` (+ `product_owner`) | 90d |
| `high` | 3 | `mitigate`, `elevate` (not acceptable) | C3 | — | 90d |

`qualitative3x3` (analogue):

```text
low    low     medium
low    medium  serious
medium serious high
```

`nasa8004_5x5` (MIL-STD-882E severity IV..I map to rows 2..5; row 1 is
one band below marginal):

```text
low    low     low     low     low
low    low     low     medium  medium
low    medium  medium  serious serious
medium serious serious high    high
medium serious high    high    high
```

`signoff.max_residual_band = "medium"`. Required-action tailoring: an
engineered prevention is required for `serious` and `high`; procedural-only
mitigation is flagged; the design order of precedence is enforced by action
ordering.

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
none   none   none      none      report
none   report report    remediate remediate
report report remediate remediate remediate
```

The S×P matrices are the `standard` layout mapped to this preset's bands
(`none` → `none`, `compensate` → `report`, `review` → `remediate`).
`signoff.max_residual_band = "report"`.

## Built-in action catalog

`include_builtin_actions = true` merges the shared catalog in
`crates/fmeca-policy/presets/actions.toml` (28 actions over the tags `design`,
`review`, `test`, `ops`, `process`, `docs` and `security`). Catalog conditions
are band-agnostic on purpose — every preset has its own band vocabulary — and
use confidence, domain, level, response class, disposition, single-point,
missing mitigation kinds, fallback and class.

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
