#!/usr/bin/env bash
set -euo pipefail

# Playbook: Verify Full Test Suite
# Runs all unit, integration, and extras tests across agent-brain.

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../.." && pwd)"

echo "==> [Playbook: Full Suite] Running cargo test -p agent-brain --all-features..."
cd "$REPO_ROOT"
cargo test -p agent-brain --all-features

echo "==> [Playbook: Full Suite] Passed cleanly (exit 0)."
