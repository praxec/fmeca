# Installing fmeca for AI coding agents

This guide is written so an AI coding agent can follow it end to end, and so a
person can read it the same way. It installs two things for your client:

1. **The MCP server** (`fmeca-mcp`), which gives the agent the FMECA tools.
2. **The skills** (nine `SKILL.md` folders), which teach the agent when and how
   to use those tools at each level: concept, conops, technical plan, project
   plan, implementation and verification.

Do the steps in order for one client. Each client section is self-contained.

## Before you start

- You need one way to run the server. Pick one and use it in every snippet below:

  | Way | Needs | Command in the MCP config |
  |-----|-------|---------------------------|
  | npx (easiest) | Node.js 18 or newer | `npx -y @matthew-cochran/fmeca` |
  | Binary | the release installer, or `cargo install fmeca-mcp` | `fmeca-mcp` |
  | Docker | Docker | `docker run -i --rm ghcr.io/praxec/fmeca:0.1.0` |

- The server is offline. The one exception is the optional `vet` tool, which
  asks Jev (through OpenRouter) for a second opinion and needs
  `OPENROUTER_API_KEY`. **`vet` is optional: every other tool works without a
  key**, and `vet` without one reports `vet_unavailable`.
- **Never write a key into a config file.** Every snippet here either refers to
  `${OPENROUTER_API_KEY}` or inherits the variable from the environment your
  client was started in. Export the key in your shell profile; do not paste it
  into JSON or TOML.

To install the binary instead of using npx:

```sh
# Linux / macOS
curl -fsSL https://github.com/praxec/fmeca/releases/download/v0.1.0/install.sh | sh -s -- --version v0.1.0
```

```powershell
# Windows (PowerShell)
irm https://github.com/praxec/fmeca/releases/download/v0.1.0/install.ps1 | iex
```

Then check it: `fmeca-mcp --version` prints `fmeca-mcp 0.1.0`.

## Which client?

| Client | Skills target | MCP config | Section |
|--------|---------------|------------|---------|
| Claude Code | `claude` | `claude mcp add`, or `.mcp.json` | [Claude Code](#claude-code) |
| Claude Desktop | none (no installer target) | `claude_desktop_config.json` | [Claude Desktop](#claude-desktop) |
| OpenAI Codex CLI | `codex` | `codex mcp add`, or `~/.codex/config.toml` | [Codex](#codex) |
| Cursor | `cursor` | `.cursor/mcp.json` | [Cursor](#cursor) |
| VS Code / GitHub Copilot | `copilot` | `.mcp.json` | [VS Code and GitHub Copilot](#vs-code-and-github-copilot) |
| Gemini CLI | `gemini` | `gemini mcp add`, or `settings.json` | [Gemini CLI](#gemini-cli) |
| Any other client that reads `AGENTS.md` | `agents-md` | the client's own MCP config | [Generic AGENTS.md](#generic-agentsmd) |

The skills installer is the same everywhere:

```sh
fmeca-mcp skills install --target <target> --user                # all your projects
fmeca-mcp skills install --target <target> --project <dir>       # one project
```

Add `--dry-run` to see what would be written, and `--force` to overwrite a skill
file you edited (otherwise it is kept and reported as skipped). It never edits
MCP configuration: after installing it prints the registration command for the
target. Without a binary on your path, run it through npx:
`npx -y @matthew-cochran/fmeca skills install --target <target> --user`.
`fmeca-mcp skills uninstall` takes the same `--target`, `--user` and
`--project` options. The per-client paths and sources are in
[the tool matrix](agents/tool-matrix.md).

## Verify any install

After registering the server, check two things. The commands below talk to the
server directly over stdio, so they work the same for every client. Swap the
first word for the command you chose (`fmeca-mcp`, `npx -y @matthew-cochran/fmeca`
or the `docker run` line).

**1. `tools/list` shows ten tools.** The eight core tools are `session.open`,
`append`, `state.get`, `risk.next`, `readiness.assess`, `report.export`,
`scoring.catalog` and `analyze`, plus `session.list` and `vet`.

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | fmeca-mcp 2>/dev/null | grep -o '"name":"[a-z_.]*"' | sort -u
```

**2. A one-shot `analyze` returns a report.** The example ships with the
skills, so you do not need the repository. Set `EXAMPLE` to the installed copy
for your client (`~/.claude/skills/fmeca/examples/one-shot-analyze.json` for
Claude Code, `~/.agents/skills/fmeca/examples/one-shot-analyze.json` for Codex,
Gemini and the generic target) and run:

```sh
EXAMPLE=~/.claude/skills/fmeca/examples/one-shot-analyze.json
{
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  jq -nc --slurpfile a "$EXAMPLE" \
    '{jsonrpc:"2.0",id:3,method:"tools/call",params:{name:"analyze",arguments:$a[0]}}'
} | fmeca-mcp | tail -n 1 | jq '.result.structuredContent // .result | {ready, risk_ranking}'
```

You should see `ready: false` and a `risk_ranking` that starts with
`fm-retry-double-charge`. In the client itself, ask: "Use the fmeca `analyze`
tool on the example in the fmeca skill" and it should return the same report.

## Claude Code

1. Install the skills:

   ```sh
   fmeca-mcp skills install --target claude --user
   ```

   They land in `~/.claude/skills/` and are used by name (`/fmeca-concept`) or
   automatically when their description matches.

2. Register the server. Pick one:

   ```sh
   # npx
   claude mcp add --transport stdio --scope user fmeca -- npx -y @matthew-cochran/fmeca
   # binary
   claude mcp add --transport stdio --scope user fmeca -- fmeca-mcp
   # Docker, analyses kept in the repo you launch from (see the .fmeca/ section)
   claude mcp add --transport stdio --scope project fmeca -- docker run -i --rm -v "$PWD:/work" -w /work -e OPENROUTER_API_KEY ghcr.io/praxec/fmeca:0.1.0
   ```

   `--scope project` writes `.mcp.json` in the project root, which you can
   commit. Written by hand it looks like this (Claude Code expands
   `${OPENROUTER_API_KEY}` from your environment):

   ```json
   {
     "mcpServers": {
       "fmeca": {
         "type": "stdio",
         "command": "npx",
         "args": ["-y", "@matthew-cochran/fmeca"],
         "env": { "OPENROUTER_API_KEY": "${OPENROUTER_API_KEY}" }
       }
     }
   }
   ```

   For the binary use `"command": "fmeca-mcp"` and `"args": []`. For Docker use
   `"command": "docker"` and
   `"args": ["run", "-i", "--rm", "-e", "OPENROUTER_API_KEY", "ghcr.io/praxec/fmeca:0.1.0"]`.

3. Restart Claude Code, run `/mcp` to see `fmeca` connected, then
   [verify](#verify-any-install).

## Claude Desktop

Claude Desktop has no skills installer target, so use the MCP server alone. Edit
`claude_desktop_config.json` (macOS: `~/Library/Application Support/Claude/`,
Windows: `%APPDATA%\Claude\`) and restart the app:

```json
{
  "mcpServers": {
    "fmeca": {
      "command": "npx",
      "args": ["-y", "@matthew-cochran/fmeca"]
    }
  }
}
```

For the binary use `"command": "fmeca-mcp"` and `"args": []`. For Docker use
`"command": "docker"` with
`"args": ["run", "-i", "--rm", "ghcr.io/praxec/fmeca:0.1.0"]`; without a mount
the state is lost when the container exits (see
[Keep analyses in your repo](#keep-analyses-in-your-repo-fmeca)).

Desktop does not expand `${...}` in this file, so no key is configured and `vet`
reports `vet_unavailable`. Everything else works. To use `vet`, use a client
that passes your environment through, such as Claude Code.

## Codex

1. Install the skills (Codex reads `~/.agents/skills/`):

   ```sh
   fmeca-mcp skills install --target codex --user
   ```

   Use them as `$fmeca-concept` or through `/skills`.

2. Register the server. Pick one:

   ```sh
   codex mcp add fmeca -- npx -y @matthew-cochran/fmeca
   codex mcp add fmeca -- fmeca-mcp
   ```

   Or edit `~/.codex/config.toml` (a project `.codex/config.toml` works for
   trusted projects). `env_vars` lists variables Codex passes through from its
   own environment, so the key is never written down:

   ```toml
   [mcp_servers.fmeca]
   command = "npx"
   args = ["-y", "@matthew-cochran/fmeca"]
   env_vars = ["OPENROUTER_API_KEY"]
   ```

   For the binary use `command = "fmeca-mcp"` and `args = []`. For Docker use
   `command = "docker"` and
   `args = ["run", "-i", "--rm", "-e", "OPENROUTER_API_KEY", "ghcr.io/praxec/fmeca:0.1.0"]`.

3. Restart Codex, run `/mcp`, then [verify](#verify-any-install).

## Cursor

1. Install the skills:

   ```sh
   fmeca-mcp skills install --target cursor --user
   ```

   Use them as `/fmeca-concept` in Agent chat.

2. Add the server to `~/.cursor/mcp.json` (global) or `.cursor/mcp.json`
   (project). Cursor requires `type`:

   ```json
   {
     "mcpServers": {
       "fmeca": {
         "type": "stdio",
         "command": "npx",
         "args": ["-y", "@matthew-cochran/fmeca"]
       }
     }
   }
   ```

   For the binary use `"command": "fmeca-mcp"` and `"args": []`. For Docker use
   `"command": "docker"` and
   `"args": ["run", "-i", "--rm", "-e", "OPENROUTER_API_KEY", "ghcr.io/praxec/fmeca:0.1.0"]`.
   The key is inherited from the environment Cursor was started in.

3. Enable `fmeca` under Settings, MCP, then [verify](#verify-any-install).

## VS Code and GitHub Copilot

1. Install the skills:

   ```sh
   fmeca-mcp skills install --target copilot --user
   ```

   They land in `~/.copilot/skills/`; use them as `/fmeca-concept` in chat.

2. Add the server to `.mcp.json` at the project root, or
   `~/.copilot/mcp-config.json` for all projects. Both use the key
   `mcpServers`. The older `.vscode/mcp.json` (key `servers`) is deprecated.

   ```json
   {
     "mcpServers": {
       "fmeca": {
         "type": "stdio",
         "command": "npx",
         "args": ["-y", "@matthew-cochran/fmeca"],
         "env": { "OPENROUTER_API_KEY": "${OPENROUTER_API_KEY}" }
       }
     }
   }
   ```

   For the binary use `"command": "fmeca-mcp"` and `"args": []`. For Docker use
   `"command": "docker"` and
   `"args": ["run", "-i", "--rm", "-e", "OPENROUTER_API_KEY", "ghcr.io/praxec/fmeca:0.1.0"]`.
   VS Code can also add it to your user profile with
   `code --add-mcp '{"name":"fmeca","command":"npx","args":["-y","@matthew-cochran/fmeca"]}'`.

3. Reload the window, then [verify](#verify-any-install).

## Gemini CLI

1. Install the skills and the `/fmeca-*` commands:

   ```sh
   fmeca-mcp skills install --target gemini --user
   ```

   Skills go to `~/.agents/skills/` and commands to `~/.gemini/commands/`.

2. Register the server (`-s user` or `-s project`; avoid underscores in the
   name):

   ```sh
   gemini mcp add -s user fmeca npx -y @matthew-cochran/fmeca
   gemini mcp add -s user fmeca fmeca-mcp
   ```

   Or edit `~/.gemini/settings.json` (project: `.gemini/settings.json`):

   ```json
   {
     "mcpServers": {
       "fmeca": {
         "command": "npx",
         "args": ["-y", "@matthew-cochran/fmeca"],
         "env": { "OPENROUTER_API_KEY": "${OPENROUTER_API_KEY}" }
       }
     }
   }
   ```

   For the binary use `"command": "fmeca-mcp"` and `"args": []`. For Docker use
   `"command": "docker"` and
   `"args": ["run", "-i", "--rm", "-e", "OPENROUTER_API_KEY", "ghcr.io/praxec/fmeca:0.1.0"]`.

3. Restart Gemini CLI, run `/mcp`, then [verify](#verify-any-install).

## Generic AGENTS.md

For any client that reads `AGENTS.md` and has its own MCP configuration:

```sh
fmeca-mcp skills install --target agents-md --project .
```

This writes `.agents/skills/` and a managed block in `./AGENTS.md` between
`<!-- fmeca-mcp:begin -->` and `<!-- fmeca-mcp:end -->`; everything outside the
markers is left as it was. There is no user-level `AGENTS.md`, so `--user` is
refused with this target (use `--target codex --user`). Register the server in
your client's MCP configuration using the stdio command from the table in
[Before you start](#before-you-start), then [verify](#verify-any-install).

## Keep analyses in your repo (`.fmeca/`)

By default the server keeps one append-only log per analysis in your user data
directory, which no teammate can see. Keep them in the repository instead, so
analyses are reviewed, merged and versioned with the code.

1. At the project root, run `fmeca-mcp init` (or `fmeca-mcp init --dir <path>`).
   It creates `.fmeca/README.md` and `.fmeca/.gitattributes` (the latter sets a
   union merge so two branches that append to the same log both keep their
   events). **Commit `.fmeca/`.**
2. **The server finds `.fmeca/` by walking up from its working directory.**
   Launch the MCP server from the project root, or set `FMECA_STATE_DIR` to the
   folder. The state directory is resolved in this order: `FMECA_STATE_DIR`,
   else the nearest `.fmeca/` above the working directory, else the user data
   directory (`$XDG_DATA_HOME/fmeca-mcp` or `$HOME/.local/share/fmeca-mcp`),
   else `./fmeca-state`.
3. **Name each session `<aspect>-<level>`**, with `_` in the level written as
   `-`: `checkout-technical-plan`. One aspect, one chain of levels, each linked
   to the one above with `parent`.
4. **Find existing analyses with `session.list`** before opening a new one. It
   filters by `level`, `aspect` and `parent`, and shows standing counts, `ready`
   and `ready_verified` for each. For example:

   ```json
   { "level": "technical_plan", "aspect": "checkout" }
   ```

5. **Track resolution as work lands.** Move a mitigation through `planned`,
   `implemented`, `verified` and `abandoned` with `append` and
   `set_mitigation_status` (`verified` needs evidence other than a note). Accept
   a residual risk (needs a named approver) or exclude a failure mode as not
   credible (needs evidence) with `set_failure_mode_disposition`. Failure modes
   are never deleted. The `fmeca-resolve` skill does this.
6. **Check it in CI.** Add a step that replays every log and fails on a problem:

   ```sh
   fmeca-mcp check --dir .fmeca
   ```

   A union merge can keep both branches' events and so duplicate an id, or leave
   a log that no longer replays; `check` names the log and the problem.

### Docker and the repo

A container only sees `.fmeca/` if the project is mounted. Use this as the
in-repo variant:

```sh
docker run -i --rm -v "$PWD:/work" -w /work -e OPENROUTER_API_KEY ghcr.io/praxec/fmeca:0.1.0
```

The `-w /work` makes the project root the working directory, so the server finds
`.fmeca/` as above. **Without the mount, state goes to the container's own user
directory and is lost when the container exits.** The container runs as a
non-root user (uid 10001), so the mounted folder must be writable by that user.

### Levels and skills

| Level | Question | Skill |
|-------|----------|-------|
| `concept` | Is this worth building, and who can it hurt? | [fmeca-concept](../skills/fmeca-concept/SKILL.md) |
| `conops` | How will the business run it? | [fmeca-concept](../skills/fmeca-concept/SKILL.md) |
| `technical_plan` | How will we build it? | [fmeca-technical-plan](../skills/fmeca-technical-plan/SKILL.md) |
| `project_plan` | How will we deliver it? | [fmeca-project-plan](../skills/fmeca-project-plan/SKILL.md) |
| `implementation` | What can fail in the code? | [fmeca-implementation](../skills/fmeca-implementation/SKILL.md) |
| `verification` | What did we prove? | [fmeca-verification](../skills/fmeca-verification/SKILL.md) |

Around the levels: [fmeca](../skills/fmeca/SKILL.md) is the method reference,
[fmeca-resolve](../skills/fmeca-resolve/SKILL.md) records progress,
[fmeca-vet](../skills/fmeca-vet/SKILL.md) asks for the optional Jev second
opinion, and [fmeca-signoff](../skills/fmeca-signoff/SKILL.md) produces the
sign-off summary. The level detail is in
[levels.md](../skills/fmeca/levels.md).

## Environment variables

| Variable | Default | Effect |
|----------|---------|--------|
| `FMECA_STATE_DIR` | nearest `.fmeca/`, else the user data dir | Where the session logs live. |
| `OPENROUTER_API_KEY` | unset | Enables `vet`. Read from the environment only; never put it in a file. |
| `FMECA_JEV_MODEL` | `typesafe/jev-1.13` | The model `vet` asks. |
| `FMECA_JEV_TIMEOUT_SECS` | `30` | Per-request timeout for `vet`, in seconds. |

## Troubleshooting

- **`tools/list` is empty or the client says the server failed to start.** Run
  the command from the config in a terminal. `npx` needs Node 18 or newer and
  network access on first run.
- **Native Windows: `npx` not found.** Some clients need
  `"command": "cmd"` with `"args": ["/c", "npx", "-y", "@matthew-cochran/fmeca"]`.
- **The skills do not show up.** Restart the client. Cursor and Copilot can list
  a skill twice if you install more than one target; that is harmless.
- **`vet` returns `vet_unavailable`.** No key reached the server. Export
  `OPENROUTER_API_KEY` in the environment the client starts from. This does not
  affect any other tool.
- **Analyses are missing the next day.** The server was not started where it
  can see `.fmeca/`, or a Docker run had no mount. See
  [Keep analyses in your repo](#keep-analyses-in-your-repo-fmeca).
