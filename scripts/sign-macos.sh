#!/usr/bin/env bash
# Adhoc-sign agent-brain after cargo build (macOS only).
# Linker-signed release binaries are rejected by taskgated when Cursor launches MCP.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/target/release/agent-brain}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "sign-macos.sh: skipped (not macOS)" >&2
  exit 0
fi

if [[ ! -f "$BIN" ]]; then
  echo "sign-macos.sh: binary not found: $BIN" >&2
  exit 1
fi

xattr -cr "$BIN"
codesign --force --sign - "$BIN"
codesign --verify --verbose "$BIN"

echo "Signed $BIN (adhoc)"
