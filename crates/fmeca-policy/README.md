# fmeca-policy

Declarative risk policies for [fmeca](https://crates.io/crates/fmeca). A
policy is a TOML document that re-colours fmeca's fixed criticality matrices
into a project's own risk bands, sets a sign-off rule, and suggests tagged
actions. `evaluate(subject, policy, as_of)` is a pure function: no network, no
clock, and the same inputs always give byte-identical JSON. fmeca suggests; it
never acts.

The crate owns the policy model, TOML parsing, validation, the canonical hash,
loading project policies from `.fmeca/policies/<name>.toml`, the evaluator
(confidence ladder C0–C4, low-confidence fallback, dispositions, actions,
roll-ups, sign-off) and markdown rendering. The `fmeca-mcp` server exposes it
as the `actions.suggest` and `policy.list` tools and the `suggest` /
`policy validate` / `policy list` commands.

## Presets

| Preset | Bands (low → high) | Sign-off |
|---|---|---|
| `compat-0.1` (default) | `none` / `mitigate` | equals the fmeca 0.1 readiness verdict |
| `prototype` | `none` / `report` / `remediate` | max `report` |
| `standard` | `none` / `compensate` / `review` | max `compensate` |
| `business-critical` | as `standard`, tighter evidence and tailoring | max `compensate` |
| `safety-critical` | `low` / `medium` / `serious` / `high` (MIL-STD-882E) | max `medium` |
| `confidence-view` | `none` / `report` / `remediate` over band × confidence | max `report` |

The full model, every preset's cells, the confidence ladder, the fallback
rule and the built-in action catalog are in
[docs/policies.md](https://github.com/praxec/fmeca/blob/dev/docs/policies.md).

## License

Apache-2.0.
