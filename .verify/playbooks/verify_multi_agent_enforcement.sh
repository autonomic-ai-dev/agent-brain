#!/usr/bin/env bash
set -euo pipefail

# Playbook: Verify Multi-Agent Enforcement & Gate Interception (ABH-16)

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../.." && pwd)"

echo "==> [Playbook: Multi-Agent Enforcement] Running Python hook & AST safety test suite..."
cd "$REPO_ROOT"
PYTHONDONTWRITEBYTECODE=1 python3 tekmor/hooks/test_route_gate.py

echo "==> [Playbook: Multi-Agent Enforcement] Running Rust host_hooks unit tests..."
cargo test -p tekmor --lib host_hooks

echo "==> [Playbook: Multi-Agent Enforcement] Running Rust MCP route gate unit tests..."
cargo test -p tekmor --lib mcp::route_gate

echo "==> [Playbook: Multi-Agent Enforcement] Running multi-agent harness simulation suite..."
PYTHONDONTWRITEBYTECODE=1 python3 tekmor/hooks/simulate_multi_agent_session.py

echo "==> [Playbook: Multi-Agent Enforcement] Passed cleanly (exit 0)."
