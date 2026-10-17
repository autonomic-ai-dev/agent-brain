#!/usr/bin/env bash
set -euo pipefail

# agent-brain verification suite entrypoint
# Executed by `agent-brain verify -- .verify/run.sh`

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/.." && pwd)"

echo "==> [agent-brain] Starting verification suite..."
cd "$REPO_ROOT"

echo "==> Running full unit and integration test suite..."
cargo test -p agent-brain --all-features

echo "==> Verifying cargo check with zero warnings..."
cargo check -p agent-brain --all-features

echo "==> [agent-brain] Verification completed successfully (exit 0)."
