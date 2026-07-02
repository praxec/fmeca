# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-06-22

Initial release.

### Added

- Deterministic, offline FMECA kernel (`fmeca`): an append-only event log
  folded into computed state — criticality, residual risk, standing, and
  readiness are never stored, only derived.
- Fixed S×P criticality matrix and a fixed observations→score map: the caller
  supplies *observations*, never scores; the code derives every level.
- Deterministic `response_class` remediation-magnitude derivation.
- Prevent→detect→fail-fast mitigation discipline, with issue detection and
  computed residual risk.
- A readiness gate reporting whether every High/Medium residual has been
  reduced and every blocker cleared.
- Swappable matrix strategies, selectable at `session.open`: the default 3×3
  qualitative matrix and the NASA GSFC-HDBK-8004 5×5 matrix. A session's
  strategy is fixed once opened and recorded for deterministic replay.
- 8-tool stdio MCP surface (`fmeca-mcp`): `session.open`, `append`, `state.get`,
  `risk.next`, `readiness.assess`, `report.export`, `scoring.catalog`, and the
  stateless `analyze` batch tool, which reuses the same kernel compute as the
  session path so the two cannot diverge.

### Known limitations

- Residual risk and scores derive from a fixed qualitative scoring catalog; the
  catalog is code-resident and not caller-extensible.

[0.0.1]: https://github.com/praxec/fmeca/releases/tag/v0.0.1
