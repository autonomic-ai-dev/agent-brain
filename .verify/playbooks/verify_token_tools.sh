#!/usr/bin/env bash
set -euo pipefail

# Playbook: Verify Token Tools & Bounded Reads
# Validates bounded file readers, grep searches, and token saving benchmarks.

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../.." && pwd)"

echo "==> [Playbook: Token Tools] Testing bounded reads, file summary, and token benchmarks..."
cd "$REPO_ROOT"
cargo test -p agent-brain --all-features token_tools::tests

echo "==> [Playbook: Token Tools] Passed cleanly (exit 0)."
