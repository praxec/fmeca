#!/usr/bin/env bash
# Smoke test for `fmeca-mcp skills install|uninstall` against a real binary, in
# throwaway directories. Used by the skills-smoke job in .github/workflows/ci.yml
# on Linux, macOS and Windows (Git Bash).
#
# The embedded skill set is whatever is under the workspace skills/ directory.
# With an empty (or absent) skills/ the run only asserts exit 0 and that a
# rerun reports nothing but `unchanged`; files are asserted once skills/ holds
# skills.
#
# Usage: scripts/skills-smoke.sh <path-to-fmeca-mcp> [<scratch-dir>]
# The scratch dir defaults to $RUNNER_TEMP, else a new mktemp -d directory.
set -euo pipefail

exe="${1:?usage: skills-smoke.sh <path-to-fmeca-mcp> [<scratch-dir>]}"
scratch="${2:-${RUNNER_TEMP:-$(mktemp -d)}}"
scratch="$(cd "$scratch" && pwd)"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# A path as the native binary should see it (C:\... on Windows).
native() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}

fail() { echo "FAIL: $*" >&2; exit 1; }

# The skill names the binary embeds: each skills/<name>/SKILL.md in the workspace.
skills=""
commands=""
if [ -d "$repo/skills" ]; then
  for f in "$repo"/skills/*/SKILL.md; do
    [ -f "$f" ] || continue
    name="$(basename "$(dirname "$f")")"
    skills="$skills $name"
    case "$name" in fmeca-*) commands="$commands $name" ;; esac
  done
fi
if [ -z "$skills" ]; then
  echo "skills/ is empty: asserting exit codes and idempotent output only"
fi

# Every skill's SKILL.md exists under each given skills directory.
expect_skills() {
  for dir in "$@"; do
    for s in $skills; do
      [ -f "$dir/$s/SKILL.md" ] || fail "missing $dir/$s/SKILL.md"
    done
  done
}

# No file is left under the given directory. The last uninstall of a skills
# root also removes its manifest. Lock files are never deleted: they live in the
# per-user cache directory (under the redirected HOME here), so that directory is ignored.
expect_no_files() {
  left="$(find "$1" -type f ! -path "*/fmeca-mcp/locks/*" -print)"
  [ -z "$left" ] || fail "files left after uninstall: $left"
}

# Lines that report one file: "<status>  <path>".
file_lines() { grep -E '^(created|updated|unchanged|modified, skipped|foreign, skipped|removed)  ' "$@" || true; }

# Every file line in the given logs says `unchanged`; at least one when skills/ is non-empty.
expect_all_unchanged() {
  total="$(cat "$@" | file_lines | wc -l | tr -d ' ')"
  changed="$(cat "$@" | file_lines | grep -v '^unchanged  ' || true)"
  if [ -n "$skills" ]; then
    [ "$total" -gt 0 ] || fail "rerun printed no file lines"
  fi
  [ -z "$changed" ] || fail "rerun changed files: $changed"
  echo "all $total file lines unchanged"
}

echo "== project scope, every target"
proj="$scratch/proj"
rm -rf "$proj" && mkdir -p "$proj"
for t in claude codex cursor copilot gemini agents-md; do
  "$exe" skills install --target "$t" --project "$(native "$proj")" > "$scratch/first-$t.log"
done
cat "$scratch"/first-*.log
expect_skills "$proj/.claude/skills" "$proj/.agents/skills" "$proj/.cursor/skills" "$proj/.github/skills"
for c in $commands; do
  [ -f "$proj/.gemini/commands/$c.toml" ] || fail "missing $proj/.gemini/commands/$c.toml"
done
[ -f "$proj/AGENTS.md" ] || fail "missing $proj/AGENTS.md"

echo "== project rerun is a no-op"
for t in claude codex cursor copilot gemini; do
  "$exe" skills install --target "$t" --project "$(native "$proj")" > "$scratch/second-$t.log"
done
expect_all_unchanged "$scratch"/second-*.log

echo "== user scope (HOME and USERPROFILE redirected, path with a space)"
uhome="$scratch/home with space"
rm -rf "$uhome" && mkdir -p "$uhome"
user_env=(env "HOME=$(native "$uhome")" "USERPROFILE=$(native "$uhome")")
for t in claude codex cursor copilot gemini; do
  "${user_env[@]}" "$exe" skills install --target "$t" --user > "$scratch/user-$t.log"
done
expect_skills "$uhome/.claude/skills" "$uhome/.agents/skills" "$uhome/.cursor/skills" "$uhome/.copilot/skills"
for c in $commands; do
  [ -f "$uhome/.gemini/commands/$c.toml" ] || fail "missing $uhome/.gemini/commands/$c.toml"
done

echo "== user rerun is a no-op"
for t in claude codex cursor copilot gemini; do
  "${user_env[@]}" "$exe" skills install --target "$t" --user > "$scratch/user-rerun-$t.log"
done
expect_all_unchanged "$scratch"/user-rerun-*.log

if [ "${RUNNER_OS:-}" = "Windows" ] || [ "${OS:-}" = "Windows_NT" ]; then
  echo "== Windows: USERPROFILE alone (no HOME)"
  wprofile="$scratch/profile only"
  rm -rf "$wprofile" && mkdir -p "$wprofile"
  env -u HOME "USERPROFILE=$(native "$wprofile")" "$exe" skills install --target claude --user > "$scratch/profile.log"
  expect_skills "$wprofile/.claude/skills"
  env -u HOME "USERPROFILE=$(native "$wprofile")" "$exe" skills uninstall --target claude --user > "$scratch/profile-uninstall.log"
  expect_no_files "$wprofile"
fi

echo "== uninstall"
for t in claude codex cursor copilot gemini agents-md; do
  "$exe" skills uninstall --target "$t" --project "$(native "$proj")" > "$scratch/uninstall-$t.log"
done
expect_no_files "$proj"
for t in claude codex cursor copilot gemini; do
  "${user_env[@]}" "$exe" skills uninstall --target "$t" --user > "$scratch/user-uninstall-$t.log"
done
expect_no_files "$uhome"

echo "skills smoke: OK"
