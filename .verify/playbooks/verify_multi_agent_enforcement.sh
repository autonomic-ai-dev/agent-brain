#!/usr/bin/env bash
set -euo pipefail

# Playbook: Verify Multi-Agent Enforcement & Gate Interception (ABH-16)
# Validates route_gate.py hook logic, mouth AST safety policy, multi-agent tool extractors,
# and Rust host_hooks + MCP route gate modules.

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../.." && pwd)"

echo "==> [Playbook: Multi-Agent Enforcement] Running Python hook & AST safety test suite..."
cd "$REPO_ROOT"
python3 agent-brain/hooks/test_route_gate.py

echo "==> [Playbook: Multi-Agent Enforcement] Running Rust host_hooks unit tests..."
cargo test -p agent-brain --lib host_hooks

echo "==> [Playbook: Multi-Agent Enforcement] Running Rust MCP route gate unit tests..."
cargo test -p agent-brain --lib mcp::route_gate

echo "==> [Playbook: Multi-Agent Enforcement] Passed cleanly (exit 0)."
