# fmeca-mcp

[![crates.io](https://img.shields.io/crates/v/fmeca-mcp.svg)](https://crates.io/crates/fmeca-mcp)

The stdio MCP host for [`fmeca`](../fmeca): a thin server that exposes
the deterministic FMECA kernel as a command/query MCP tool surface plus a
stateless `analyze` batch tool. Every handler parses args, calls one `Engine`
method, and serializes the result — all structure and state live in the kernel.

```sh
cargo install fmeca-mcp
```

```jsonc
{ "command": "fmeca-mcp" }
```

See the root [README](../../README.md) for the MCP client config, the full tool
surface, and the design rationale.

## License

[Apache-2.0](../../LICENSE).
