# fmeca

[![crates.io](https://img.shields.io/crates/v/fmeca.svg)](https://crates.io/crates/fmeca)
[![docs.rs](https://docs.rs/fmeca/badge.svg)](https://docs.rs/fmeca)

The deterministic, offline kernel for structured **FMECA** (Failure Modes,
Effects & Criticality Analysis).

The caller supplies *observations* — never scores. This kernel owns the
structure and the arithmetic: a typed failure-mode / mitigation ledger over an
append-only event log, a fixed evidence→score map, a locked S×P criticality
matrix (swappable between a 3×3 qualitative default and the NASA GSFC-HDBK-8004
5×5), the prevent→detect→fail-fast mitigation discipline, computed residual
risk, issue detection, and a readiness gate. Criticality, residual, and standing
are *computed* by a fold over the log — never stored. No LLM, no network.

## Stateless one-shot

```rust
use fmeca::{analyze, AnalyzeInput, Domain, EntityRef, MatrixStrategy};

let failure_modes = vec![AnalyzeInput {
    id: "fm-1".into(),
    component: EntityRef::new("checkout-service"),
    description: "Payment double-charge on retry".into(),
    cause: None,
    effect: None,
    domain: Domain::Runtime,
    scope: None,
    severity_observations: vec!["data_loss".into()],
    probability_observations: vec!["happens_in_normal_use".into()],
    mitigations: vec![],
}];

let report = analyze(MatrixStrategy::default(), &failure_modes).unwrap();
println!("risk ranking: {:?}", report.risk_ranking);
```

## Long-lived sessions

For multi-turn analyses, use `Engine` over a `StateStore` (a `FilesystemStore`
is provided). It persists an append-only event log and replays it to compute
state, so restart survival is free:

```rust
use std::sync::Arc;
use fmeca::{Engine, FilesystemStore};

let store = Arc::new(FilesystemStore::new("./fmeca-state"));
let engine = Engine::new(store);
let state = engine.open_session("design-review").unwrap();
println!("ready: {}", state.readiness.ready);
```

See the root [README](../../README.md) for the MCP server and the full design
rationale.

## License

[Apache-2.0](../../LICENSE).
