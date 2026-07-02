# Security Policy

## Reporting a vulnerability

Please report security issues privately via GitHub's
[security advisories](https://github.com/praxec/fmeca/security/advisories/new)
rather than a public issue. You'll get an acknowledgement, and a fix or
mitigation will be coordinated before public disclosure.

## Scope

fmeca-mcp is a deterministic, offline MCP server that speaks over stdio and holds
FMECA session state in its own process (an append-only event log on the local
filesystem). It executes no user-supplied code, makes no network calls, and
shells out to nothing — its inputs are failure modes, mitigations, and
observation ids. Of particular interest: any input (a crafted `analyze` payload,
`append` sequence, or session id) that causes a panic, a hang, a path-traversal
write outside the state directory, or a computed criticality/residual/readiness
that diverges from the fixed scoring map and matrix.
