# Contributing to fmeca-mcp

Thanks for your interest. fmeca-mcp is a small, focused workspace: a
deterministic FMECA kernel plus a thin MCP server façade. This guide covers the
toolchain, the crate map, the gates, the branch flow, and how to run the
end-to-end checks.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). For
questions about contributing, see [SUPPORT.md](SUPPORT.md).

## Toolchain

- The pinned toolchain in [`rust-toolchain.toml`](rust-toolchain.toml) is
  **1.99.0** (with `rustfmt` and `clippy`). `rustup` installs it automatically
  when you run Cargo in the repository.
- The **MSRV** (minimum supported Rust version) is **1.85** for the `fmeca`
  kernel, recorded as `rust-version` in the workspace manifest; `fmeca-mcp`
  sets **1.89**. Do not use language or library
  features newer than 1.85 without raising the MSRV deliberately.

## Skills and packaging

`fmeca-mcp` embeds the workspace `skills/` directory at build time (`build.rs`
reads `$CARGO_MANIFEST_DIR/skills` if it exists, otherwise `../../skills`; with
neither, or an empty directory, it embeds nothing). `cargo package` cannot reach
`../../skills`, so the `package` CI job first copies `skills/` into
`crates/fmeca-mcp/skills/`, which is gitignored, and the packaged crate embeds
that copy. Do the same copy before a local `cargo package` or `cargo publish`.
Install runs serialise on an OS file lock per target root. The lock files live
in the per-user cache directory (`$XDG_CACHE_HOME/fmeca-mcp/locks`, else
`~/.cache/fmeca-mcp/locks`; `~/Library/Caches/...` on macOS;
`%LOCALAPPDATA%\fmeca-mcp\locks` on Windows), are named by a hash of the
canonical root path, and are never deleted.
`fmeca-mcp` therefore needs Rust 1.89 (`File::try_lock`); it sets its own
`rust-version`.

## Crate map

| Crate | Role |
|-------|------|
| `fmeca` | The pure kernel: the event log, scoring, criticality matrix, residual risk, readiness, and the stateless `analyze` function. No I/O beyond the event-log store and no network access. |
| `fmeca-mcp` | The MCP server and CLI: the stdio transport, tool surface, session wiring, the `skills` installer, and Jev. It sits on top of `fmeca` and adds no scoring logic of its own. |

Keep the boundary clean: computation belongs in `fmeca`; parsing, transport,
and presentation belong in `fmeca-mcp`.

## Gates

Every pull request must pass these gates locally before review:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

`cargo deny check` (configuration in [`deny.toml`](deny.toml)) enforces the
licence, advisory, and source policy. CI also runs `cargo fmt`, `clippy`
(warnings denied), and the test matrix on Linux, macOS, and Windows, plus the
packaging and installer jobs.

## Branch flow

Work flows in one direction:

```
feature/* → dev → main
```

- Branch off `dev` with a descriptive name (`feature/...`, `fix/...`,
  `docs/...`).
- Open pull requests against `dev`, not `main`.
- `dev` is the integration branch; `main` only receives releases merged from
  `dev`. Dependabot targets `dev` for the same reason.
- Keep commits conventional and focused (`feat:`, `fix:`, `docs:`, `ci:`, …).
  Update `CHANGELOG.md` under `[Unreleased]`.

## End-to-end checks

Build the server, then exercise the real MCP handshake:

```sh
cargo build
node scripts/mcp-smoke.mjs target/debug/fmeca-mcp
```

The smoke script uses only Node builtins, makes no API calls, and sends the
`initialize` and `tools/list` requests over stdio.

Jev's evaluation path talks to OpenRouter and is therefore ignored by default.
To run it live (this spends money and needs a real key), export the key and opt
in explicitly. The `vet` tests, the live one included, compile only with the
`test-fakes` feature:

```sh
OPENROUTER_API_KEY=… cargo test -p fmeca-mcp --features test-fakes -- --ignored
```

Never commit a key or paste one into an issue.

## Test discipline

- The kernel is deterministic: computed state is a pure fold over the event log,
  and the scoring map and criticality matrix are fixed. Assert exact values, not
  just shapes.
- The exception is the ignored Jev live tests; everything in the default suite
  must be deterministic and make no network calls.
- Prefer one behavioural assertion per test. Name tests after the behaviour they
  pin down, and assert through the public interface rather than private helpers.
- The caller supplies observations, never scores. Do not add a path that lets a
  caller hand in a criticality, residual, or readiness directly.
- Add a test for any behaviour change, and fix the behaviour rather than the
  test.
