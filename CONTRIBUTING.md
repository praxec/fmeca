# Contributing to fmeca-mcp

Thanks for your interest. fmeca-mcp is a small, focused workspace — a
deterministic FMECA kernel (`fmeca`) plus a thin MCP server façade
(`fmeca-mcp`).

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

## Guidelines

- Keep the two concerns clean: the pure kernel (`fmeca`) has no I/O beyond
  its event-log store; the MCP server (`fmeca-mcp`) parses args and serializes
  results on top of it.
- The kernel is deterministic — computed state is a pure fold over the event
  log, and the scoring map and criticality matrix are fixed. Add a test for any
  behavior change; the tests should be deterministic too.
- The caller supplies observations, never scores. Don't add a path that lets a
  caller hand in a criticality, residual, or readiness directly.
- Conventional, focused commits.

## Reporting issues

Open an issue with a minimal reproduction — for scoring or readiness bugs, the
`analyze` payload (or the sequence of `append` calls) that produces the wrong
result is ideal.
