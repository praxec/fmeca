# Agent tool matrix for `fmeca-mcp skills install --target <tool>`

Ported from the cpm-planner matrix (researched 2026-10-10) and **re-checked against each tool's current official docs on 2026-10-11**. Every row below names the URL fetched and the check date. The installer writes only the paths listed under "Writes". It never edits MCP configuration files: the registration command is printed instead.

Two findings drive the design:

1. **Agent Skills (`<dir>/<name>/SKILL.md`, agentskills.io) is the common format.** Claude Code, Codex, Cursor, VS Code Copilot and Gemini CLI all load it; only the directory differs.
2. **`.agents/skills/` (project) and `~/.agents/skills/` (user) are the closest thing to a shared location.** Codex, Cursor, Copilot and Gemini CLI read them. **Claude Code does not**: it reads only `.claude/skills/` and `~/.claude/skills/`. So `--target all` writes `.claude/skills` and `.agents/skills` only.

## Shared skill file

`<skills-root>/<name>/SKILL.md` with front matter `name` (1-64 characters of `[a-z0-9-]`, equal to the directory name) and `description` (1-1024 characters). Supporting files in the skill directory are allowed. `build.rs` enforces these rules at build time for every embedded skill.

The skills that ship are the `fmeca` reference skill and the `fmeca-*` workflow skills. Only the `fmeca-*` skills also get a Gemini CLI command.

## Per-tool rows

| Target | Writes (project) | Writes (user) | Invocation users get | Official source (checked 2026-10-11) |
|---|---|---|---|---|
| `claude` | `.claude/skills/<skill>/SKILL.md` | `~/.claude/skills/<skill>/SKILL.md` | `/<skill>`; also auto-loaded by description | https://code.claude.com/docs/en/skills |
| `codex` | `.agents/skills/<skill>/SKILL.md` | `~/.agents/skills/<skill>/SKILL.md` | `$<skill>` or `/skills` (no `/<skill>`) | https://learn.chatgpt.com/docs/build-skills (308 redirect from https://developers.openai.com/codex/skills) |
| `cursor` | `.cursor/skills/<skill>/SKILL.md` | `~/.cursor/skills/<skill>/SKILL.md` | `/<skill>` in Agent chat | https://cursor.com/docs/context/skills |
| `copilot` | `.github/skills/<skill>/SKILL.md` | `~/.copilot/skills/<skill>/SKILL.md` | `/<skill>` in chat; `/skills` opens the menu | https://code.visualstudio.com/docs/copilot/customization/agent-skills |
| `gemini` | `.agents/skills/<skill>/SKILL.md` and `.gemini/commands/fmeca-*.toml` | `~/.agents/skills/<skill>/SKILL.md` and `~/.gemini/commands/fmeca-*.toml` | `/<fmeca-skill>` from the TOML command; skills are model-activated via `activate_skill` | https://geminicli.com/docs/cli/skills/ and https://geminicli.com/docs/cli/custom-commands/ |
| `agents-md` | managed block in `./AGENTS.md` and `.agents/skills/<skill>/SKILL.md` | none (usage error; use `codex --user`) | tool-dependent; AGENTS.md alone gives no command | https://agents.md/ |
| `all` | `.claude/skills` and `.agents/skills` | same, under `~` | as `claude` plus `codex` | rows above |

Duplicates: Cursor reads `.claude`, `.codex` and `.agents` skills, and Copilot reads `.claude` and `.agents`, so installing several targets can list a skill twice. No tool documents dedup, so `--target all` stays at two roots and prints a warning.

## 1. Claude Code

- Skills: user `~/.claude/skills/<name>/SKILL.md`, project `.claude/skills/<name>/SKILL.md` (checked: https://code.claude.com/docs/en/skills, 2026-10-11). `.agents/skills` is not documented. `.claude/commands/` still works, and a skill wins on a name clash.
- Invocation: `/<name> [args]`; `$ARGUMENTS` expands the arguments.
- MCP (printed, never written): `claude mcp add --transport stdio --scope <local|project|user> <name> -- <command> [args...]`. Scope `project` writes `.mcp.json`, `user` writes `~/.claude.json`. `--` separates Claude's options from the server command (checked: https://code.claude.com/docs/en/mcp, 2026-10-11).
- AGENTS.md: read natively from v2.1.277, **only when no `CLAUDE.md`, `.claude/CLAUDE.md` or `CLAUDE.local.md` exists** in the working directory or above; otherwise import it with `@AGENTS.md` (checked: https://code.claude.com/docs/en/memory, 2026-10-11).

## 2. OpenAI Codex CLI

- Skills: project `$CWD/.agents/skills` (and parents up to the repo root), user `$HOME/.agents/skills`, admin `/etc/codex/skills`. `~/.codex/skills` is not documented (checked: https://learn.chatgpt.com/docs/build-skills, 2026-10-11).
- Invocation: `$<name>` or `/skills`; implicit when the description matches. There is no `/<name>` for skills.
- MCP (printed): `codex mcp add <name> [--env K=V] -- <command> [args...]`; config in `~/.codex/config.toml`, or project `.codex/config.toml` for trusted projects only, as `[mcp_servers.<name>]` with `command` and `args` (checked: https://learn.chatgpt.com/docs/extend/mcp?surface=cli, 2026-10-11).
- AGENTS.md: global `~/.codex` (`AGENTS.override.md` then `AGENTS.md`), then root-to-cwd concatenation capped at `project_doc_max_bytes` (32 KiB) (checked: https://learn.chatgpt.com/docs/agent-configuration/agents-md, 2026-10-11).

## 3. Cursor

- Skills: project `.agents/skills/` and `.cursor/skills/`, user `~/.agents/skills/` and `~/.cursor/skills/`. Cursor also reads `.claude/skills` and `.codex/skills` (checked: https://cursor.com/docs/context/skills, 2026-10-11). `name` must match the folder.
- Invocation: `/<name>` in Agent chat.
- MCP (printed): `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` (global), key `mcpServers`, `type: "stdio"` required (checked: https://cursor.com/docs/context/mcp, 2026-10-11).
- AGENTS.md: supported at the root and in subdirectories, with more specific files taking precedence (checked: https://cursor.com/docs/context/rules, 2026-10-11).
- `.cursor/commands` has no current official documentation, so the installer does not write it.

## 4. Gemini CLI

- Commands: user `~/.gemini/commands/`, project `<root>/.gemini/commands/`; `fmeca-x.toml` becomes `/fmeca-x`; `prompt` is required, `description` optional; `{{args}}` is replaced by the typed text (checked: https://geminicli.com/docs/cli/custom-commands/, 2026-10-11).
- Skills: user `~/.gemini/skills/` or `~/.agents/skills/`, workspace `.gemini/skills/` or `.agents/skills/`; `.agents/` wins within a tier. Activation is model-triggered through `activate_skill` (checked: https://geminicli.com/docs/cli/skills/, 2026-10-11).
- MCP (printed): `gemini mcp add [options] <name> <commandOrUrl> [args...]`, `-s user|project` (default project); settings in `~/.gemini/settings.json` or `.gemini/settings.json`, key `mcpServers` (checked: https://geminicli.com/docs/tools/mcp-server/, 2026-10-11). Avoid underscores in server names.
- AGENTS.md: read only when `context.fileName` lists it (checked: https://geminicli.com/docs/cli/gemini-md/, 2026-10-11).
- Gemini CLI is being replaced by Antigravity CLI for consumer tiers (noted on the custom-commands page, 2026-10-11). Antigravity paths are not written by the installer.

## 5. GitHub Copilot in VS Code

- Skills: project `.github/skills/`, `.claude/skills/`, `.agents/skills/`; user `~/.copilot/skills/`, `~/.claude/skills/`, `~/.agents/skills/` (checked: https://code.visualstudio.com/docs/copilot/customization/agent-skills, 2026-10-11).
- Invocation: `/<name>` in chat.
- MCP (printed): **changed since the cpm-planner matrix.** The VS Code destinations `.vscode/mcp.json` (key `servers`) and the VS Code user-profile `mcp.json` are now marked deprecated. The preferred destinations are the portable `.mcp.json` at the project root and `~/.copilot/mcp-config.json` (or `$COPILOT_HOME/mcp-config.json`), both with key `mcpServers`. `code --add-mcp '{"name":...,"command":...}'` is still documented for the user profile (checked: https://code.visualstudio.com/docs/copilot/customization/mcp-servers, 2026-10-11).
- AGENTS.md: read from the repo root (`chat.useAgentsMdFile`); nested files are experimental and off by default (`chat.useNestedAgentsMdFiles`) (checked: https://code.visualstudio.com/docs/copilot/customization/custom-instructions, 2026-10-11).

## 6. Generic AGENTS.md

Plain Markdown with no required fields; the nearest file wins and an explicit chat prompt overrides all (checked: https://agents.md/, 2026-10-11). The installer writes a managed block between `<!-- fmeca-mcp:begin -->` and `<!-- fmeca-mcp:end -->` in project `AGENTS.md` and leaves everything outside the markers byte-identical. There is no standard user-level AGENTS.md, so `--target agents-md --user` is a usage error.

## Changes against the cpm-planner matrix

| Item | Was (2026-10-10) | Now (2026-10-11) | Renderer change |
|---|---|---|---|
| Copilot project MCP file | `.vscode/mcp.json`, key `servers` | `.mcp.json`, key `mcpServers` (the VS Code file is deprecated) | Printed snippet uses `.mcp.json` and `mcpServers`. |
| Copilot user MCP file | VS Code profile `mcp.json` ("MCP: Open User Configuration") | `~/.copilot/mcp-config.json` or `$COPILOT_HOME/mcp-config.json`, key `mcpServers` | Printed user snippet names that file; `code --add-mcp` kept. |
| Claude Code, Codex, Cursor, Gemini skill paths, invocation and MCP commands | as above | unchanged | none |

## Not re-checked for this release

Windsurf, Zed, Cline, Roo Code, Amp and JetBrains Junie were summarised in the cpm-planner matrix (2026-10-10). The installer has no target for them. Those that read `.agents/skills` (Zed, Roo, Amp, Windsurf) are reached by `--target codex`; Cline and Junie are not. Re-verify before adding a dedicated target.

## Name rule

Keep `name` equal to the directory and `[a-z0-9-]{1,64}`. The `fmeca` reference skill and the `fmeca-*` skills all comply.
