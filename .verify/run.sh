#!/usr/bin/env bash
set -euo pipefail

# agent-brain verification suite entrypoint
# Executed by `agent-brain verify -- .verify/run.sh`

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/.." && pwd)"

echo "==> [agent-brain] Starting verification suite..."
cd "$REPO_ROOT"

echo "==> Verifying cargo check with zero warnings..."
cargo check -p agent-brain

echo "==> Running Playbook: Turn Telemetry & Cache Savings..."
"$DIR/playbooks/verify_usage_telemetry.sh"

echo "==> Running Playbook: Token Tools..."
"$DIR/playbooks/verify_token_tools.sh"

echo "==> Running Playbook: Routing..."
"$DIR/playbooks/verify_routing.sh"

echo "==> Running Playbook: Multi-Agent Enforcement & Interception..."
"$DIR/playbooks/verify_multi_agent_enforcement.sh"

echo "==> Running Playbook: Full Suite..."
"$DIR/playbooks/verify_full_suite.sh"

echo "==> [agent-brain] Verification completed successfully (exit 0)."
