#!/usr/bin/env bash
# junior-contract-check.sh — assert the release wiring and MCP tool surface
# that a Junior build must preserve. Each violation fails with a named message.
#
# Usage: scripts/junior-contract-check.sh [path-to-built-fmeca-mcp]
#   default binary: target/debug/fmeca-mcp
# Env overrides (used by the negative self-test):
#   RELEASE_YML        release workflow (default .github/workflows/release.yml)
#   RELEASE_MANIFEST_SH  manifest script (default scripts/release-manifest.sh)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="${1:-target/debug/fmeca-mcp}"
release_yml="${RELEASE_YML:-$root/.github/workflows/release.yml}"
manifest_sh="${RELEASE_MANIFEST_SH:-$root/scripts/release-manifest.sh}"

fail() { echo "junior contract: $*" >&2; exit 1; }

# 1. The release binary name is pinned.
grep -q '^  BIN: fmeca-mcp$' "$release_yml" \
  || fail "release.yml no longer pins 'BIN: fmeca-mcp'"

# 2. Every frozen target/ext pair is still in the release matrix, and the
#    package step still derives the asset name from BIN/TARGET/EXT.
while read -r triple ext; do
  [ -n "$triple" ] || continue
  grep -q "{ target: $triple, .*ext: $ext }" "$release_yml" \
    || fail "release.yml matrix row missing for $triple ($ext)"
done <<'MATRIX'
x86_64-unknown-linux-gnu tar.gz
aarch64-unknown-linux-gnu tar.gz
x86_64-apple-darwin tar.gz
aarch64-apple-darwin tar.gz
x86_64-pc-windows-msvc zip
aarch64-pc-windows-msvc zip
MATRIX

grep -qF 'asset="${BIN}-${TARGET}.${EXT}"' "$release_yml" \
  || fail "release.yml no longer builds asset=\"\${BIN}-\${TARGET}.\${EXT}\""

# 3. The checksum wiring is intact: the release uploads dist/checksums.sha256
#    and the manifest script writes '<hex>  <name>' lines.
grep -q 'dist/checksums.sha256' "$release_yml" \
  || fail "release.yml no longer uploads dist/checksums.sha256"
grep -q 'sha256' "$manifest_sh" \
  || fail "release-manifest.sh no longer mentions sha256"
grep -qF '%s  %s' "$manifest_sh" \
  || fail "release-manifest.sh no longer writes two-space '<hex>  <name>' lines"

# 4. The built server still advertises the eight pinned MCP tool names.
[ -x "$bin" ] || fail "built fmeca-mcp not found or not executable: $bin"
tools="$(
  FMECA_BIN="$bin" node --input-type=module <<'NODE'
import { spawn } from 'node:child_process';

const bin = process.env.FMECA_BIN;
const child = spawn(bin, [], { stdio: ['pipe', 'pipe', 'pipe'] });
let buffer = '';
const pending = new Map();
let stderrText = '';
child.stderr.on('data', (d) => { stderrText += d.toString(); });
child.stdout.on('data', (chunk) => {
  buffer += chunk.toString();
  let idx;
  while ((idx = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, idx).trim();
    buffer = buffer.slice(idx + 1);
    if (!line) continue;
    let msg;
    try { msg = JSON.parse(line); } catch { continue; }
    if (msg.id != null && pending.has(msg.id)) {
      const resolve = pending.get(msg.id);
      pending.delete(msg.id);
      resolve(msg);
    }
  }
});

function request(id, method, params) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`timed out waiting for ${method}: ${stderrText.trim()}`)),
      15000,
    );
    pending.set(id, (msg) => { clearTimeout(timer); resolve(msg); });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
}

try {
  const init = await request(1, 'initialize', {
    protocolVersion: '2025-06-18',
    capabilities: {},
    clientInfo: { name: 'junior-contract', version: '1.0.0' },
  });
  if (init.error) throw new Error('initialize returned error: ' + JSON.stringify(init.error));
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized', params: {} }) + '\n');
  const list = await request(2, 'tools/list', {});
  if (list.error) throw new Error('tools/list returned error: ' + JSON.stringify(list.error));
  const tools = (list.result && list.result.tools) || [];
  for (const tool of tools) console.log(tool.name);
  child.kill();
} catch (error) {
  child.kill();
  console.error('junior contract: MCP tools probe failed: ' + error.message);
  process.exit(1);
}
NODE
)" || fail "built fmeca-mcp did not answer initialize + tools/list"

for name in session.open append state.get risk.next readiness.assess report.export scoring.catalog analyze; do
  printf '%s\n' "$tools" | grep -qx "$name" \
    || fail "tools/list no longer advertises '$name'"
done

echo "junior contract ok"
