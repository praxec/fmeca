# fmeca-mcp

[![CI](https://github.com/praxec/fmeca/actions/workflows/ci.yml/badge.svg)](https://github.com/praxec/fmeca/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/fmeca-mcp.svg)](https://crates.io/crates/fmeca-mcp)
[![docs.rs](https://docs.rs/fmeca/badge.svg)](https://docs.rs/fmeca)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

fmeca-mcp is a structured **FMECA** engine exposed as an MCP server. You feed it
failure modes and the observations that characterise them; it derives the
severity/probability scores, computes criticality off a fixed matrix, tracks
mitigations under a prevent→detect→fail-fast discipline, and tells you whether
the analysis is ready to sign off. Any MCP client (Claude Code, Cursor, or a
custom orchestrator) drives it over the standard protocol.

## What is FMECA

**Failure Modes, Effects & Criticality Analysis** is a structured risk-triage
method: enumerate how a thing can fail, judge how bad each failure is (severity)
and how likely it is (probability), and rank the combined **criticality** so the
worst risks get addressed first. Mitigations are then layered on and the
*residual* risk is re-assessed. fmeca-mcp owns the structure and the arithmetic
of that process; the caller supplies the domain judgement.

## Design principle: deterministic, offline, no LLM-supplied numbers

The engine is a pure function of `(events in) → (criticality + residual + gaps +
readiness out)`. It does no I/O beyond an append-only event log, makes no network
calls, and runs no model.

Crucially, **the caller never supplies a score.** A caller (often an LLM) does
the fuzzy work — naming a failure mode, its cause and effect, and proposing
mitigations — and supplies *observations* drawn from a fixed catalog (e.g.
`data_loss`, `happens_in_normal_use`). The kernel maps those observations to
severity/probability levels through a locked evidence→score map and reads
criticality off a fixed, locked S×P matrix. The model proposes; the code scores.
Because criticality, residual risk, standing, and readiness are *computed* by a
fold over the event log — never stored, never accepted as input — the same
analysis always yields the same result, and no model can talk the matrix into a
softer answer.

## Install

From crates.io:

```sh
cargo install fmeca-mcp
```

Or download a pre-built binary for your platform from the
[latest release](https://github.com/praxec/fmeca/releases/latest)
(verify against the release's `checksums.sha256`):

| Platform | Download |
|----------|----------|
| Linux x86_64 | [`.tar.gz`](https://github.com/praxec/fmeca/releases/latest/download/fmeca-mcp-x86_64-unknown-linux-gnu.tar.gz) |
| Linux ARM64 | [`.tar.gz`](https://github.com/praxec/fmeca/releases/latest/download/fmeca-mcp-aarch64-unknown-linux-gnu.tar.gz) |
| macOS x86_64 | [`.tar.gz`](https://github.com/praxec/fmeca/releases/latest/download/fmeca-mcp-x86_64-apple-darwin.tar.gz) |
| macOS Apple Silicon | [`.tar.gz`](https://github.com/praxec/fmeca/releases/latest/download/fmeca-mcp-aarch64-apple-darwin.tar.gz) |
| Windows x86_64 | [`.zip`](https://github.com/praxec/fmeca/releases/latest/download/fmeca-mcp-x86_64-pc-windows-msvc.zip) |

## MCP client config

It speaks MCP over stdio (the standard transport). Wire it into your editor like
any other MCP server:

```jsonc
{ "command": "fmeca-mcp" }
```

Session state is persisted as one JSONL event log per session under the state
directory, resolved from `FMECA_STATE_DIR`, else `$XDG_DATA_HOME/fmeca-mcp` (or
`$HOME/.local/share/fmeca-mcp`), else `./fmeca-state`.

## Tool surface

| Tool | Does |
|------|------|
| `session.open` | Start or resume a session; optionally selects the matrix strategy (fixed for the session's life). |
| `append` | Add a failure mode, add a mitigation, or re-score an existing failure mode. Returns the recomputed state. |
| `state.get` | Read-only projection of the session: every failure mode with its computed criticality, residual, and standing. |
| `risk.next` | The highest-criticality unmitigated failure mode — what to address next. |
| `readiness.assess` | The readiness gate: is every High/Medium residual reduced and every blocker cleared? |
| `report.export` | The full FMECA table for the session. |
| `scoring.catalog` | The fixed observation vocabulary for a strategy — the ids the caller draws from. |
| `analyze` | Stateless one-shot: hand over an entire analysis, get back the entire computed report. No session, no persistence. |

The `analyze` tool reuses the exact same kernel compute as the session path, so
the two can never diverge.

## Matrix strategies

The criticality matrix is a closed, code-resident strategy selected at
`session.open` (and fixed for the session's life). Each strategy ships with a
locked S×P matrix and its own scoring catalog:

| Strategy | Scale | Notes |
|----------|-------|-------|
| `qualitative_3x3` | Low / Medium / High | The default 3×3 qualitative matrix. |
| `nasa_8004_5x5` | 5×5 consequence × likelihood | The NASA GSFC-HDBK-8004 risk matrix. |

The caller *selects* a strategy; it never edits a strategy's cells.

## Use `fmeca` as a library

The kernel is a plain Rust library, independent of MCP. The stateless `analyze`
function takes a strategy and a batch of failure modes and returns the full
computed report:

```rust
use fmeca::{
    analyze, AnalyzeInput, AnalyzeMitigation, Domain, EntityRef, MatrixStrategy, MitigationKind,
};

let failure_modes = vec![AnalyzeInput {
    id: "fm-1".into(),
    component: EntityRef::new("checkout-service"),
    description: "Payment double-charge on retry".into(),
    cause: Some("Non-idempotent charge endpoint".into()),
    effect: Some("Customer billed twice".into()),
    domain: Domain::Runtime,
    scope: None,
    // Observations, not scores — the kernel derives severity/probability.
    severity_observations: vec!["data_loss".into()],
    probability_observations: vec!["happens_in_normal_use".into()],
    mitigations: vec![AnalyzeMitigation {
        id: "m-1".into(),
        kind: MitigationKind::Prevention,
        description: "Idempotency key on the charge endpoint".into(),
        residual_severity_observations: vec![],
        residual_probability_observations: vec!["rare_edge_case".into()],
    }],
}];

let report = analyze(MatrixStrategy::default(), &failure_modes).unwrap();
println!("risk ranking: {:?}", report.risk_ranking);
println!("ready: {} blockers: {:?}", report.ready, report.blockers);
// report.failure_modes carries per-FM criticality / residual_criticality /
// standing / response_class / issues.
```

For long-lived, multi-turn analyses use `fmeca::Engine` over a
`StateStore` (a `FilesystemStore` is provided), which persists an append-only
event log and replays it to compute state. See the
[API docs](https://docs.rs/fmeca).

## Development

```sh
cargo build
cargo test
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

CI runs build + test on Linux/macOS/Windows, plus `rustfmt` and `clippy`
(warnings are denied). Please make sure those pass locally before opening a pull
request.

## License

[Apache-2.0](LICENSE).
