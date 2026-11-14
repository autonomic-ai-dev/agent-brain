#!/usr/bin/env bash
export TEKMOR_READ_GATE="${TEKMOR_READ_GATE:-hard}"
export TEKMOR_ROUTE_OFFLINE_SECS="${TEKMOR_ROUTE_OFFLINE_SECS:-300}"
export TEKMOR_ROUTE_GATE_SCOPE="${TEKMOR_ROUTE_GATE_SCOPE:-all}"
exec python3 "$(dirname "$0")/route_gate.py" "$@"
