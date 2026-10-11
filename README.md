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

**For AI agents:** to install the server and its skills for your client, follow
[docs/AGENT-INSTALL.md](docs/AGENT-INSTALL.md).

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

### npx (no install step)

```sh
npx -y @matthew-cochran/fmeca
```

The [npm launcher](npm/README.md) downloads the matching release binary,
verifies its checksum, caches it and runs it. Point your MCP client at that
command (see [MCP client config](#mcp-client-config)).

### Prebuilt binary (no Rust, Cargo, or Git required)

```sh
# Linux / macOS
curl -fsSL https://github.com/praxec/fmeca/releases/latest/download/install.sh | sh

# Windows (PowerShell)
irm https://github.com/praxec/fmeca/releases/latest/download/install.ps1 | iex
```

The installer resolves your OS and CPU architecture, downloads the matching
release asset, verifies its SHA-256 against the release's `checksums.sha256`,
extracts it safely, and atomically installs the binary into a user-local
managed directory (`$HOME/.local/bin` on Linux/macOS,
`%LOCALAPPDATA%\Programs\fmeca-mcp` on Windows). It never compiles from source
and fails loudly on an unsupported OS/architecture.

Pin a specific release instead of the current latest stable:

```sh
curl -fsSL https://github.com/praxec/fmeca/releases/latest/download/install.sh \
  | sh -s -- --version v0.1.0
```

The installers can also be fetched from a specific release, for example
`https://github.com/praxec/fmeca/releases/download/v0.1.0/install.sh`.

The installer scripts are published as release assets alongside the binaries.

Or download the archive directly. Every release publishes a `checksums.sha256`
and a machine-readable `release-manifest.json` listing each target, asset,
digest, version, and source SHA:

| OS | Arch | Target triple | Asset | Build support | Runtime smoke tested |
|----|------|---------------|-------|---------------|----------------------|
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `fmeca-mcp-x86_64-unknown-linux-gnu.tar.gz` | native CI | native CI |
| Linux | arm64 | `aarch64-unknown-linux-gnu` | `fmeca-mcp-aarch64-unknown-linux-gnu.tar.gz` | native CI | native CI |
| macOS | x86_64 | `x86_64-apple-darwin` | `fmeca-mcp-x86_64-apple-darwin.tar.gz` | native CI | native CI |
| macOS | Apple Silicon | `aarch64-apple-darwin` | `fmeca-mcp-aarch64-apple-darwin.tar.gz` | native CI | native CI |
| Windows | x86_64 | `x86_64-pc-windows-msvc` | `fmeca-mcp-x86_64-pc-windows-msvc.zip` | native CI | native CI |
| Windows | arm64 | `aarch64-pc-windows-msvc` | `fmeca-mcp-aarch64-pc-windows-msvc.zip` | native CI | native CI |

"Native CI" means each asset is built and its MCP `initialize`/`tools/list`
handshake smoke-tested on that platform's own runner. No other CPU/OS
combination is claimed; unsupported platforms are rejected rather than
silently cross-compiled.

### Updates

Re-run the installer to update. Only the binary in the managed install
directory is replaced (atomically); application state and configuration live
outside that directory and are never overwritten. See
[Configuration](#configuration) for where state goes.

### Docker

```sh
docker run -i --rm ghcr.io/praxec/fmeca:0.1.0
```

Mount your project to keep analyses in its `.fmeca/` folder (without a mount
the state is lost when the container exits):

```sh
docker run -i --rm -v "$PWD:/work" -w /work -e OPENROUTER_API_KEY ghcr.io/praxec/fmeca:0.1.0
```

### From source

```sh
cargo install fmeca-mcp
```

`cargo install` works once the crate is published to crates.io. Until then,
build from a checkout with `cargo build --release`.

## MCP client config

It speaks MCP over stdio (the standard transport). Wire it into your editor like
any other MCP server. The same entry works in Claude Code (`.mcp.json`), Cursor
(`.cursor/mcp.json`) and Copilot (`.mcp.json`):

```json
{
  "mcpServers": {
    "fmeca": {
      "command": "npx",
      "args": ["-y", "@matthew-cochran/fmeca"]
    }
  }
}
```

Use `"command": "fmeca-mcp"` with `"args": []` for the binary. Codex uses TOML:

```toml
[mcp_servers.fmeca]
command = "npx"
args = ["-y", "@matthew-cochran/fmeca"]
env_vars = ["OPENROUTER_API_KEY"]
```

Every client, with the skills install for each, is in
[docs/AGENT-INSTALL.md](docs/AGENT-INSTALL.md). The key is optional (only `vet`
uses it) and is never written to a config file: pass it as
`"env": { "OPENROUTER_API_KEY": "${OPENROUTER_API_KEY}" }` where the client
expands variables, or let the server inherit it from the environment.

## Tool surface

| Tool | Does |
|------|------|
| `session.open` | Start or resume a session; optionally selects the matrix strategy and records analysis metadata (`title`, `level`, `aspect`, `scope_paths`, `parent`), all fixed for the session's life. |
| `append` | Add a failure mode (`add_failure_mode`), add a mitigation (`add_mitigation`), or re-score an existing failure mode (`rescore`). `set_mitigation_status` moves a mitigation through planned, implemented, verified or abandoned (verified needs evidence other than a note). `set_failure_mode_disposition` accepts a failure mode's residual risk (needs an approver) or excludes it as not credible (needs evidence); failure modes are never deleted. Returns the recomputed state. |
| `state.get` | Read-only projection of the session: every failure mode with its computed criticality, residual, and standing. |
| `risk.next` | The highest-criticality unmitigated failure mode — what to address next. |
| `readiness.assess` | The readiness gate: is every High/Medium residual reduced and every blocker cleared? |
| `report.export` | The full FMECA table for the session. |
| `scoring.catalog` | The fixed observation vocabulary for a strategy — the ids the caller draws from. |
| `analyze` | Stateless one-shot: hand over an entire analysis, get back the entire computed report. No session, no persistence. |
| `session.list` | Every analysis in the state dir (the project's `.fmeca/` when there is one), with its metadata, standing counts, `ready`, `ready_verified` and resolution counts. Filters: `level`, `aspect`, `parent`. A log that fails to replay is listed with its error. |
| `vet` | Advisory: asks Jev (via OpenRouter, needs `OPENROUTER_API_KEY`) whether observations, mitigations, coverage and readiness hold up, and returns probabilities beside the deterministic gate. Never writes or changes a score; without a key it reports `vet_unavailable`. |

The `analyze` tool reuses the exact same kernel compute as the session path, so
the two can never diverge.

## Keep analyses in your repo (`.fmeca/`)

```sh
fmeca-mcp init                 # at the project root: creates .fmeca/
git add .fmeca && git commit -m "chore: add .fmeca"
fmeca-mcp check --dir .fmeca   # in CI: replays every log, catches duplicate ids
```

Launch the server from the project root (or set `FMECA_STATE_DIR`) and it keeps
one log per analysis in `.fmeca/`, named `<aspect>-<level>`. The setup, merge
behaviour, Docker mount and the skill for each level are in
[docs/AGENT-INSTALL.md](docs/AGENT-INSTALL.md#keep-analyses-in-your-repo-fmeca).

## Configuration

| Variable | Default | Effect |
|----------|---------|--------|
| `FMECA_STATE_DIR` | nearest `.fmeca/`, else `$XDG_DATA_HOME/fmeca-mcp` (or `$HOME/.local/share/fmeca-mcp`), else `./fmeca-state` | Where the per-session logs live. |
| `OPENROUTER_API_KEY` | unset | Enables the optional `vet` tool. Read from the environment only. |
| `FMECA_JEV_MODEL` | `typesafe/jev-1.13` | The model `vet` asks. |
| `FMECA_JEV_TIMEOUT_SECS` | `30` | `vet` request timeout, in seconds. |

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

The agent skills under `skills/` are embedded in the binary at build time. After
adding or removing a skill, a local incremental build does not notice; run
`touch crates/fmeca-mcp/build.rs` and build again.

CI runs build + test on Linux/macOS/Windows, plus `rustfmt` and `clippy`
(warnings are denied). Please make sure those pass locally before opening a pull
request.

## Using it with Praxec

This is an MCP tool used by [Praxec](https://github.com/praxec/praxec) packs. The easiest way to
get it — and a workflow pack that uses it — up and running is the one-command setup:

```bash
curl -fsSL https://raw.githubusercontent.com/praxec/packs/main/setup.sh | bash
```

See the [pack registry](https://github.com/praxec/packs) for this tool's provider coordinates
(container image / release binary) and which packs depend on it.

## License

[Apache-2.0](LICENSE).
