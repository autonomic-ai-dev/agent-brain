#!/usr/bin/env bash
# Sign a macOS release binary.
# Prefers Developer ID Application when APPLE_IDENTITY is set or present in the keychain;
# falls back to ad-hoc signing (codesign -s -) for local/dev builds.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${1:-$ROOT/target/release/agent-brain}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "sign-macos.sh: skipped (not macOS)" >&2
  exit 0
fi


if [[ -z "$BIN" || ! -f "$BIN" ]]; then
  echo "sign-macos.sh: binary not found: ${BIN:-<empty>}" >&2
  echo "Usage: $0 <path-to-binary>" >&2
  exit 1
fi

xattr -cr "$BIN" 2>/dev/null || true

IDENTITY="${APPLE_IDENTITY:-}"
if [[ -z "$IDENTITY" ]]; then
  IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null     | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p'     | head -1 || true)"
fi

if [[ -n "$IDENTITY" ]]; then
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$BIN"
  codesign --verify --verbose "$BIN"
  echo "Signed $BIN with $IDENTITY"
else
  codesign --force --sign - "$BIN"
  codesign --verify --verbose "$BIN"
  echo "Signed $BIN (adhoc)"
fi
