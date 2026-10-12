# Security Policy

## Supported versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | ✅        |

Only the latest `0.1.x` release receives security fixes. Older versions are
unsupported; please upgrade before reporting.

## Reporting a vulnerability

Please report security issues privately via GitHub's
[security advisories](https://github.com/praxec/fmeca/security/advisories/new)
rather than a public issue. Include a minimal reproduction and the version
output of `fmeca-mcp --version`.

We aim to:

- **acknowledge** your report within **3 business days**, and
- **triage** it (severity, affected versions, and a fix or mitigation plan)
  within **10 business days**.

Please give us a chance to release a fix before public disclosure. We will
credit reporters who wish to be named.

## Scope

fmeca-mcp is a deterministic, offline MCP server that speaks over stdio. Its
persistent state is one append-only event log per analysis (session), under the
resolved state directory: `FMECA_STATE_DIR`, else the nearest `.fmeca/`
directory above the working directory, else the user data directory
(`$XDG_DATA_HOME/fmeca-mcp`, `$HOME/.local/share/fmeca-mcp`, or
`./fmeca-state`). `skills install` also writes small per-user lock files under
the user cache directory. Its inputs are failure modes, mitigations, and observation ids.

Of particular interest:

- The event logs in the state directory: any input (a crafted `analyze`
  payload, `append` sequence, or session id) that causes a panic, a hang, or a
  path-traversal write outside the state directory.
- `skills install`: the installer must write only under the selected project or
  user directory, never elsewhere on the filesystem.
- Computed results: any criticality, residual, or readiness that diverges from
  the fixed scoring map and matrix.

### Outbound network access: `vet`

The server is offline by default. The single exception is the optional `vet`
operation, which can make one outbound HTTPS call to OpenRouter to ask an
evaluation model to classify evidence. That call happens only when `vet` is
invoked and `OPENROUTER_API_KEY` is present.

Key hygiene for that path:

- The key is read from the environment only (`OPENROUTER_API_KEY`); it is never
  written to disk or to a configuration file.
- The key is never logged, echoed, or included in errors or telemetry. Request
  and error text is scrubbed before it is surfaced.

If you find a path where the key can leak into logs, state, or output, treat it
as a security vulnerability and report it privately.
