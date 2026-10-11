# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Kernel (`fmeca`): analysis metadata and links. `SessionMeta` (title, level,
  aspect, scope paths, parent session) is recorded on `SessionOpened`, with
  `AnalysisLevel` covering concept, conops, technical plan, project plan,
  implementation and verification. A failure mode may name a `parent` failure
  mode it refines and carry free-form `refs`. Parent links are checked on
  append, never on replay. `Engine::open_session_with_meta`,
  `Engine::list_sessions` and `StateStore::list_sessions` are new.
- Kernel: mitigation resolution lifecycle. `Engine::set_mitigation_status`
  records `planned | implemented | verified | abandoned` with evidence and an
  optional `basis`. A `verified` status needs evidence that is not a note.
  Each mitigation view gains `status`, `evidence` and `history`. Each failure
  mode gains `verified_residual_criticality`.
- Kernel: failure-mode disposition. `Engine::set_failure_mode_disposition`
  records `open | accepted | excluded`. Accepted risks need an approver and
  stop blocking the gates. Excluded failure modes need non-note evidence and
  leave `risk.next`, the buckets and the gates. Every change is kept in
  `disposition_history`.
- Kernel: three-level effects (`local_effect` and `next_effect`; the existing
  `effect` is the end effect) and `single_point`, which feeds the new
  `critical_items` in readiness and export.
- Kernel: readiness gains `resolution` counts, a `verified` gate computed from
  verified mitigations only, `accepted_risks`, `excluded` and
  `critical_items`. Export rows gain the same fields.
- Kernel: `validate_log` reports structural problems in a session log, such as
  the duplicate ids a git union-merge produces. `project` stays total over
  such logs.

### Changed

- An `abandoned` mitigation no longer counts toward residual risk, standing,
  the unmitigated detectors or `risk.next`. This only affects logs that use
  the new status event.

### Compatibility

- Old logs replay unchanged; every new field has a serde default, and a
  session without metadata is written byte-identically.
- Forward compatibility: logs that use the new `mitigation_status_changed` or
  `failure_mode_dispositioned` events cannot be read by fmeca 0.0.2 or
  earlier.

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
