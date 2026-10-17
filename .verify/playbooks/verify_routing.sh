#!/usr/bin/env bash
set -euo pipefail

# Playbook: Verify Routing & Adherence
# Validates route_task, verified recipes, must_apply elevation, and context feedback.

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../.." && pwd)"

echo "==> [Playbook: Routing & Adherence] Testing route_task, verify recipes, and must_apply elevation..."
cd "$REPO_ROOT"
cargo test -p agent-brain --all-features verify::tests
cargo test -p agent-brain --all-features engine::tests

echo "==> [Playbook: Routing & Adherence] Passed cleanly (exit 0)."
