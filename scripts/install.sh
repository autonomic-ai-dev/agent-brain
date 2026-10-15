#!/usr/bin/env bash
# Install agent-brain MCP server and register it with Cursor.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/autonomic-ai-dev/agent-brain/master/scripts/install.sh | bash
#   curl -fsSL ... | bash -s -- --from-source
#   curl -fsSL ... | bash -s -- --global
#
set -euo pipefail

REPO="${AGENT_BRAIN_REPO:-autonomic-ai-dev/agent-brain}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
FROM_SOURCE=0
GLOBAL=0
PRINT_ONLY=0
WITH_STARTER=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from-source) FROM_SOURCE=1; shift ;;
    --global) GLOBAL=1; shift ;;
    --print-only) PRINT_ONLY=1; shift ;;
    --with-starter) WITH_STARTER=1; shift ;;
    -h|--help)
      cat <<'EOF'
Install agent-brain for Cursor MCP.

Options:
  --from-source   Build with cargo instead of downloading a release binary
  --global        Write ~/.cursor/mcp.json (default: ./.cursor/mcp.json)
  --print-only    Print MCP config JSON without writing files
  --with-starter  After install, run: agent-brain add @starter
  --help          Show this help

Environment:
  INSTALL_DIR     Binary install location (default: ~/.local/bin)
  AGENT_BRAIN_REPO  GitHub repo (default: autonomic-ai-dev/agent-brain)
EOF
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$os-$arch" in
    # Release artifacts are Apple Silicon only for now (signed at CI build time).
    darwin-arm64|darwin-aarch64) echo "aarch64-apple-darwin" ;;
    *) echo "unsupported" ;;
  esac
}

artifact_name() {
  local target="$1"
  echo "agent-brain-${target}"
}

# Release binaries are Developer ID–signed at build. Install only clears quarantine
# and verifies the signature — it never re-signs.
prepare_macos_release_binary() {
  local bin="$1"
  if [[ "$(uname -s)" != "Darwin" ]] || [[ ! -f "$bin" ]]; then
    return 0
  fi
  if ! command -v codesign >/dev/null 2>&1; then
    echo "ERROR: macOS requires Xcode Command Line Tools to run agent-brain under Cursor." >&2
    echo "  xcode-select --install" >&2
    return 1
  fi
  xattr -cr "$bin" 2>/dev/null || true
  if ! codesign --verify --verbose "$bin" >/dev/null 2>&1; then
    echo "ERROR: codesign verify failed for $bin" >&2
    echo "  Re-download the release artifact (it must be Developer ID–signed at build)." >&2
    return 1
  fi
  local cs_out
  cs_out="$(codesign -dv --verbose=2 "$bin" 2>&1 || true)"
  if ! echo "$cs_out" | grep -q 'Authority=Developer ID Application:'; then
    echo "ERROR: $bin is not Developer ID–signed" >&2
    echo "  Release artifacts are signed in CI; install does not re-sign." >&2
    echo "  For local builds use: make release-macos" >&2
    return 1
  fi
  echo "macOS: cleared quarantine; verified Developer ID signature on $bin"
}

install_from_release() {
  local target asset url tmp
  target="$(detect_target)"
  if [[ "$target" == "unsupported" ]]; then
    echo "Unsupported platform for release install." >&2
    echo "Shipped artifacts: aarch64-apple-darwin only (for now)." >&2
    echo "Use --from-source, or: make release-macos && cp target/release/agent-brain ~/.local/bin/" >&2
    exit 1
  fi

  asset="$(artifact_name "$target")"
  url="https://github.com/${REPO}/releases/latest/download/${asset}"

  mkdir -p "$INSTALL_DIR"
  tmp="$(mktemp)"
  echo "Downloading ${url} ..."
  if ! curl -fsSL "$url" -o "$tmp"; then
    echo "Release download failed for ${asset} (${target})." >&2
    echo "Try: bash -s -- --from-source" >&2
    echo "Or:  make release-macos" >&2
    rm -f "$tmp"
    exit 1
  fi
  chmod +x "$tmp"
  mv "$tmp" "${INSTALL_DIR}/agent-brain"
  prepare_macos_release_binary "${INSTALL_DIR}/agent-brain"
  echo "Installed to ${INSTALL_DIR}/agent-brain"
}

install_from_cargo() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Install Rust from https://rustup.rs or download a release binary." >&2
    exit 1
  fi
  echo "Building from source. Prefer: make release-macos (Developer ID–signs the binary)." >&2
  cargo install --git "https://github.com/${REPO}" --locked --force agent-brain
  local bin
  bin="$(command -v agent-brain)"
  # From-source is linker-signed unless the user signed separately — verify only.
  prepare_macos_release_binary "$bin" || {
    echo "Hint: plain cargo install is not Developer ID–signed. Run make release-macos and install that binary." >&2
    exit 1
  }
}

ensure_path() {
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
      echo "Add ${INSTALL_DIR} to your PATH, e.g.:"
      echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
      ;;
  esac
}

main() {
  if [[ "$FROM_SOURCE" -eq 1 ]]; then
    install_from_cargo
  else
    if ! install_from_release; then
      echo "Release download failed; trying --from-source ..." >&2
      install_from_cargo
    fi
  fi

  ensure_path

  local bin
  if command -v agent-brain >/dev/null 2>&1; then
    bin="$(command -v agent-brain)"
  else
    bin="${INSTALL_DIR}/agent-brain"
  fi

  prepare_macos_release_binary "$bin"

  local args=(install)
  [[ "$GLOBAL" -eq 1 ]] && args+=(--global)
  [[ "$PRINT_ONLY" -eq 1 ]] && args+=(--print-only)

  echo "Configuring Cursor MCP ..."
  "$bin" "${args[@]}"

  if [[ "$WITH_STARTER" -eq 1 && "$PRINT_ONLY" -eq 0 ]]; then
    echo "Installing starter skill pack (@starter) ..."
    "$bin" add @starter || echo "Warning: starter pack install failed (network/git). Run: agent-brain add @starter" >&2
  fi

  if [[ "$PRINT_ONLY" -eq 0 ]]; then
    echo ""
    echo "✓ agent-brain installed"
    echo "  Route ~500 tokens from thousands of skills — hooks enforce routing."
    if [[ "$WITH_STARTER" -eq 1 ]]; then
      echo "  Starter pack (@starter) — see output above."
    else
      echo "  Next: agent-brain add @starter"
    fi
    echo "  Then: restart Cursor · enable MCP · agent-brain onboarding"
    if [[ "$(uname -s)" == "Darwin" ]]; then
      echo "  macOS: if MCP is blocked, run: agent-brain doctor --fix"
    fi
  fi
}

main "$@"
