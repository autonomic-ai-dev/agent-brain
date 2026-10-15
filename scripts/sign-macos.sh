#!/usr/bin/env bash
# Sign a macOS release binary with Developer ID Application only.
# Requires APPLE_IDENTITY or a Developer ID Application cert in the keychain.
# Ad-hoc signing is not used.
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

CS_OUT="$(codesign -dv --verbose=2 "$BIN" 2>&1 || true)"
if echo "$CS_OUT" | grep -q 'Authority=Developer ID Application:'; then
  echo "Already Developer ID–signed: $BIN"
  exit 0
fi

IDENTITY="${APPLE_IDENTITY:-}"
if [[ -z "$IDENTITY" ]]; then
  IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
    | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' \
    | head -1 || true)"
fi

if [[ -z "$IDENTITY" ]]; then
  echo "sign-macos.sh: no Developer ID Application identity found" >&2
  echo "  Set APPLE_IDENTITY or install a Developer ID Application certificate." >&2
  echo "  Ad-hoc signing is disabled." >&2
  exit 1
fi

codesign --force --options runtime --timestamp --sign "$IDENTITY" "$BIN"
codesign --verify --verbose "$BIN"
echo "Signed $BIN with $IDENTITY"
