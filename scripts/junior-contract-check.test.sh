#!/usr/bin/env bash
# Negative self-test for junior-contract-check.sh: renaming one frozen release
# target must make the contract check fail, and name the renamed target.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

release_yml="$tmp/release.yml"
sed 's/x86_64-unknown-linux-gnu/x86_64-unknown-linux-musl/' \
  "$root/.github/workflows/release.yml" > "$release_yml"

set +e
output="$(RELEASE_YML="$release_yml" bash "$root/scripts/junior-contract-check.sh" \
  "$root/target/debug/fmeca-mcp" 2>&1)"
status=$?
set -e

if [ "$status" -eq 0 ]; then
  echo "junior-contract-check.test: FAIL: check passed with a renamed target" >&2
  exit 1
fi
if ! printf '%s\n' "$output" | grep -q 'matrix row missing for x86_64-unknown-linux-gnu'; then
  echo "junior-contract-check.test: FAIL: failure did not name the missing target" >&2
  printf '%s\n' "$output" >&2
  exit 1
fi
echo "junior-contract-check.test: ok"
