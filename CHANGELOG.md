# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-10-20

### Changed

- Skills trigger on risk analysis and document calling the tools without a
  registered MCP server; the `fmeca` reference skill's "Calling the tools"
  section covers both a registered server and a host wrapper (#88).

## [0.2.0] - 2026-10-12

### Added

- `fmeca-policy` crate: declarative TOML risk policies, with action bands per
  matrix cell, a deterministic confidence axis (C0-C4), low-confidence
  fallback, dispositions, an action catalog and sign-off.
- Policy presets: `compat-0.1` (default, equals the 0.1 readiness verdict),
  `prototype`, `standard`, `business-critical`, `safety-critical`
  (MIL-STD-882E) and `confidence-view`.
- Server (`fmeca-mcp`): `actions.suggest` and `policy.list` MCP tools.
- CLI: `suggest`, `policy validate` and `policy list` with markdown output.
- Policy keys beyond the 0.2 design: `signoff.include_kernel_gate` (default
  `true`), `signoff.unscored_fails` (default `true`), and the conditions
  `rank_at_least` (band-agnostic, for the shared action catalog) and
  `mitigation_kinds_present`.
- Kernel gate in sign-off: `include_kernel_gate` adds the kernel's hygiene
  blockers (CLARIFY, weak mitigation order, EMPTY) but never its RESIDUAL
  zone blockers, so `max_residual_band` decides which residuals pass.
- Fallback threshold and `evidence_mode` (maintainer ruling): by default
  (`evidence_mode = "standard"`) a residual needs its own band's
  `min_confidence`; a band that sets none borrows the nearest gated band above
  it on a claimed reduction, so a claim to the lowest band is never free while
  a band above it is gated.
  The opt-in `evidence_mode = "strong"` needs the highest `min_confidence` of
  its own band and of every band the claimed reduction leaves, so a bigger
  claimed reduction never needs less evidence. No preset sets `strong`.
- `compat-0.1` sets `unscored_fails = false`, so an accepted, unscored failure
  mode passes as in 0.1 readiness, while an open one still fails (CLARIFY).
- Docs: the `fmeca-policy` skill (with a complete `policy-template.toml`),
  `docs/policies.md` and the `fmeca-policy` crate README.

### Changed

- `fmeca` (the kernel) is republished at 0.2.0 alongside `fmeca-policy`, with
  no API change.
- npm releases are published by the maintainer with `scripts/npm-release.sh`
  (2FA one-time password, no stored token), from the tarball the release
  attached. The release workflow's `npm publish` job runs only when the
  repository variable `NPM_TRUSTED_PUBLISHING` is `true`.

### Fixed

- server.json description shortened to the MCP registry's 100-character limit; CI now validates server.json against the registry.

## [0.1.0] - 2026-10-12

### Added

- CLI: `fmeca-mcp --version` prints `fmeca-mcp <version>`.
- CLI: the `.fmeca/` project store. `fmeca-mcp init` scaffolds it with a
  README and a `.gitattributes` that sets `merge=union` on the event logs;
  `fmeca-mcp check` replays every log, runs `fmeca::validate_log` over it, and
  names the duplicate ids and the second `session_opened`
  (`duplicate_session_opened`) a union merge can produce.
- CLI: `fmeca-mcp skills install|uninstall --target <claude|codex|cursor|copilot|gemini|agents-md|all> (--project <dir> | --user)`
  installs the agent skills embedded in the binary into each tool's official
  skill and command locations (manifest-tracked, `--dry-run`, `--force`, an OS
  file lock per root) and prints the MCP registration command without editing
  any MCP configuration. See `docs/agents/tool-matrix.md`.
- npm launcher `@matthew-cochran/fmeca`: downloads the checksum-verified
  release binary for the host platform (cache and OS file lock) and runs it
  over stdio, forwarding arguments intact.
- Server (`fmeca-mcp`): `session.list`, one row per analysis in the state dir,
  with optional exact-match filters. Excluded failure modes are left out of a
  row's `failure_modes` and `by_standing` counts and counted in `excluded`.
- Server (`fmeca-mcp`): the advisory `vet` tool (feature `jev`, on by
  default). It asks Jev, a calibrated judgment model on OpenRouter, four kinds
  of question about a session or a one-shot analysis: do the claimed
  observations apply (and does the next level up), how fully does each
  mitigation address its cause and is its declared kind right, which of seven
  failure classes no failure mode covers, and is the analysis sound enough to
  sign off. Each answer becomes a finding with its probability, threshold and
  verdict, beside the unchanged deterministic readiness gate. At most
  `max_questions` (default 40, hard cap 200) are sent in one call; the rest
  are listed by deterministic sha256 hash. `vet` never writes, skips excluded
  failure modes, and without `OPENROUTER_API_KEY` (or with a rejected
  `FMECA_JEV_*` setting) reports `vet_unavailable` with a reason instead of
  failing.
- Jev client (`fmeca_mcp::jev`): a `JudgmentModel` abstraction, an OpenRouter
  client, error mapping and test fakes.
- Docs: `docs/AGENT-INSTALL.md`, an install guide an AI coding agent can follow
  for Claude Code, Claude Desktop, Codex, Cursor, VS Code/Copilot, Gemini CLI
  and AGENTS.md (MCP config, skills, verification, `.fmeca/` in your repo),
  `llms.txt`, and README install, configuration and tool sections. A test checks
  that every config snippet parses and no key is spelled out.
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
  `ResolutionEvidence` rejects unknown fields, so a misspelled key such as
  `basys` is an error rather than silently dropped.
  Each mitigation view gains `status`, `evidence` and `history`. Each failure
  mode gains `verified_residual_criticality`.
- Kernel: failure-mode disposition. `Engine::set_failure_mode_disposition`
  records `open | accepted | excluded`. Accepted risks need an approver and
  stop blocking the gates. Excluded failure modes need non-note evidence and
  leave `risk.next`, the buckets and the gates, and raise no issues or
  signals. Every change is kept in
  `disposition_history`.
- Kernel: three-level effects (`local_effect` and `next_effect`; the existing
  `effect` is the end effect) and `single_point`, which feeds the new
  `critical_items` in readiness and export.
- Kernel: readiness gains `resolution` counts, a `verified` gate computed from
  verified mitigations only, `accepted_risks`, `excluded` and
  `critical_items`. Export rows gain `local_effect`, `next_effect`,
  `single_point` and `disposition`, each exported mitigation gains `status`
  and `evidence`, and the report gains `critical_items`.
- Kernel: `validate_log` reports structural problems in a session log, such as
  the duplicate ids a git union-merge produces. `project` stays total over
  such logs.
- Community: `CODE_OF_CONDUCT.md`, `SUPPORT.md`, issue forms, a pull-request
  template and `CODEOWNERS`; `CONTRIBUTING.md` and `SECURITY.md` rewritten.
- CI: supply-chain and packaging jobs (`cargo-deny`, docs, package) and
  Dependabot updates.
- Scripts: `scripts/check-version-sync.sh` pins every version field to
  `Cargo.toml`; `scripts/junior-contract-check.sh` (with its negative
  self-test) pins the release matrix, asset names, checksum wiring and the MCP
  tool surface.

### Changed

- Rust toolchain pinned to 1.99.0 and `rmcp` bumped to 3.5.
- MSRV: `fmeca-mcp` now needs Rust 1.89; the `fmeca` kernel stays at 1.85.
- State dir resolution: when `FMECA_STATE_DIR` is unset, the server and
  `fmeca-mcp check` use the nearest ancestor `.fmeca/` directory of the
  working directory, and fall back to the user data directory only when there
  is none.
- `server.json` publishes the version as the OCI identifier tag
  (`ghcr.io/praxec/fmeca:0.1.0`) instead of a `version` field, which the MCP
  registry rejects on OCI packages.
- An `abandoned` mitigation no longer counts toward residual risk, standing,
  the unmitigated detectors or `risk.next`. This only affects logs that use
  the new status event.

### Compatibility

- Old logs replay unchanged; every new field has a serde default, and a
  session without metadata is written byte-identically.
- Forward compatibility: logs that use the new `mitigation_status_changed` or
  `failure_mode_dispositioned` events cannot be read by fmeca 0.0.2 or
  earlier.

### Security

- Jev/OpenRouter credentials are read only from the environment
  (`OPENROUTER_API_KEY`, `FMECA_JEV_*`); no key is written to the event log,
  disk or output.

### Fixed

- MCP: advertise the `security` domain in the `analyze` schema and server instructions, and make `risk.next` skip accepted failure modes.

## [0.0.2] - 2026-10-05

### Added

- Shell and PowerShell installers (`scripts/install.sh`, `scripts/install.ps1`)
  that download a verified release asset and put `fmeca-mcp` on `PATH`.
- Prebuilt archives for `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`
  and `aarch64-pc-windows-msvc`, joining the three x86_64 targets.
- Per-asset checksums: `checksums.sha256` and a machine-readable
  `release-manifest.json` published with every release.

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

[0.2.1]: https://github.com/praxec/fmeca/releases/tag/v0.2.1
[0.2.0]: https://github.com/praxec/fmeca/releases/tag/v0.2.0
[0.1.0]: https://github.com/praxec/fmeca/releases/tag/v0.1.0
[0.0.2]: https://github.com/praxec/fmeca/releases/tag/v0.0.2
[0.0.1]: https://github.com/praxec/fmeca/releases/tag/v0.0.1
