# Support

Thanks for using fmeca-mcp. Here is where to get help and what to include when
you ask.

## Where to ask

- **Bug reports** — open an issue with the [bug report form][bug]. Include a
  minimal reproduction so the failure can be reproduced without guesswork.
- **Feature requests** — open an issue with the [feature request form][feature].
- **Security vulnerabilities** — do not open a public issue. Report privately
  through [GitHub security advisories][security], as described in
  [SECURITY.md](SECURITY.md).

## What to include

A useful report gives a maintainer everything needed to reproduce the problem
without asking follow-up questions:

- The exact version: run `fmeca-mcp --version` and paste the output.
- Your OS and CPU architecture (for example, Linux x86_64 or macOS aarch64).
- The MCP client or orchestrator that drives the server (Claude Code, Cursor, a
  custom client, and so on).
- A minimal event log or `analyze` input that triggers the problem. For scoring
  or readiness bugs, the shortest sequence of `append` calls, or the single
  `analyze` payload, that produces the wrong result is ideal.
- What you expected to happen and what actually happened.

## Before you file

- Check the [README](README.md) and the existing issues for an answer.
- Reduce the reproduction to the smallest input that still shows the problem.
- Do not include secrets, API keys, or private data in an issue.

[bug]: https://github.com/praxec/fmeca/issues/new?template=bug_report.yml
[feature]: https://github.com/praxec/fmeca/issues/new?template=feature_request.yml
[security]: https://github.com/praxec/fmeca/security/advisories/new
