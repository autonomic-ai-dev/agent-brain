#!/usr/bin/env bash
set -euo pipefail


DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/.." && pwd)"

echo "==> [tekmor] Starting verification suite..."
cd "$REPO_ROOT"

echo "==> Verifying cargo check with zero warnings..."
cargo check -p tekmor

echo "==> Verifying the tekmor-core boundary..."
.verify/checks/core_boundary.sh

echo "==> Running tekmor-core unit tests..."
cargo test -p tekmor-core

echo "==> Running Playbook: Turn Telemetry & Cache Savings..."
.verify/playbooks/verify_usage_telemetry.sh

echo "==> Running Playbook: Token Tools..."
.verify/playbooks/verify_token_tools.sh

echo "==> Running Playbook: Routing..."
.verify/playbooks/verify_routing.sh

echo "==> Running Playbook: Multi-Agent Enforcement & Interception..."
.verify/playbooks/verify_multi_agent_enforcement.sh

echo "==> Running Playbook: Full Suite..."
.verify/playbooks/verify_full_suite.sh

echo "==> [tekmor] Verification completed successfully (exit 0)."
