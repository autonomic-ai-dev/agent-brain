#!/usr/bin/env bash
# Download the latest GitHub release (already Developer ID–signed) and link MCP.
# Does not force package git pulls (those stay on interval_hours, default 24h).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${TEKMOR_BIN:-$HOME/.local/bin/tekmor}"

if [[ -x "$BIN" ]]; then
  exec "$BIN" update --force --mcp-only
fi

exec "$ROOT/scripts/install.sh" --global
