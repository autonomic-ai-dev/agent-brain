#!/usr/bin/env bash
# User-level hook wrapper — env for route_gate.py (hooks do not inherit mcp.json).
export AGENT_BRAIN_READ_GATE="${AGENT_BRAIN_READ_GATE:-hard}"
export AGENT_BRAIN_ROUTE_OFFLINE_SECS="${AGENT_BRAIN_ROUTE_OFFLINE_SECS:-300}"
export AGENT_BRAIN_ROUTE_GATE_SCOPE="${AGENT_BRAIN_ROUTE_GATE_SCOPE:-all}"
exec python3 "$(dirname "$0")/route_gate.py" "$@"
