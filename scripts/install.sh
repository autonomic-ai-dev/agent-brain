#!/usr/bin/env bash
# Install agent-brain MCP server and register it with Cursor.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/master/scripts/install.sh | bash
#   curl -fsSL ... | bash -s -- --from-source
#   curl -fsSL ... | bash -s -- --global
#
set -euo pipefail

REPO="${AGENT_BRAIN_REPO:-aeswibon/agent-brain}"
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
  AGENT_BRAIN_REPO  GitHub repo (default: aeswibon/agent-brain)
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
    darwin-arm64|darwin-aarch64) echo "aarch64-apple-darwin" ;;
    darwin-x86_64) echo "x86_64-apple-darwin" ;;
    linux-x86_64|linux-amd64) echo "x86_64-unknown-linux-gnu" ;;
    linux-aarch64|linux-arm64) echo "aarch64-unknown-linux-gnu" ;;
    mingw*|msys*|cygwin*) echo "x86_64-pc-windows-msvc" ;;
    *) echo "unsupported" ;;
  esac
}

artifact_name() {
  local target="$1"
  if [[ "$target" == *"windows"* ]]; then
    echo "agent-brain-${target}.exe"
  else
    echo "agent-brain-${target}"
  fi
}

sign_macos_binary() {
  local bin="$1"
  if [[ "$(uname -s)" != "Darwin" ]] || [[ ! -f "$bin" ]]; then
    return 0
  fi
  xattr -cr "$bin"
  codesign --force --sign - "$bin"
}

install_from_release() {
  local target asset url tmp
  target="$(detect_target)"
  if [[ "$target" == "unsupported" ]]; then
    echo "Unsupported platform. Use --from-source or install Rust and run:" >&2
    echo "  cargo install --git https://github.com/${REPO} agent-brain" >&2
    exit 1
  fi

  asset="$(artifact_name "$target")"
  url="https://github.com/${REPO}/releases/latest/download/${asset}"

  mkdir -p "$INSTALL_DIR"
  tmp="$(mktemp)"
  echo "Downloading ${url} ..."
  if ! curl -fsSL "$url" -o "$tmp"; then
    echo "Release download failed for ${asset} (${target})." >&2
    if [[ "$target" == "aarch64-unknown-linux-gnu" ]]; then
      echo "Linux ARM64 binaries ship from v0.14.0+. Until then, use --from-source." >&2
    fi
    echo "Try: bash -s -- --from-source" >&2
    echo "Or:  cargo install --git https://github.com/${REPO} agent-brain" >&2
    rm -f "$tmp"
    exit 1
  fi
  chmod +x "$tmp"
  mv "$tmp" "${INSTALL_DIR}/agent-brain"
  sign_macos_binary "${INSTALL_DIR}/agent-brain"
  echo "Installed to ${INSTALL_DIR}/agent-brain"
}

install_from_cargo() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Install Rust from https://rustup.rs or download a release binary." >&2
    exit 1
  fi
  cargo install --git "https://github.com/${REPO}" --locked --force agent-brain
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
    install_from_release || install_from_cargo
  fi

  ensure_path

  local bin
  if command -v agent-brain >/dev/null 2>&1; then
    bin="$(command -v agent-brain)"
  else
    bin="${INSTALL_DIR}/agent-brain"
  fi

  sign_macos_binary "$bin"

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
  fi
}

main "$@"
