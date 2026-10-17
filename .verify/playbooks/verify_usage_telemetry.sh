#!/usr/bin/env bash
set -euo pipefail

echo "==> Running agent-brain turn telemetry and pricing tests..."
cargo test -p agent-brain --lib telemetry::tests -- --nocapture

echo "==> Running agent-brain turn telemetry store persistence tests..."
cargo test -p agent-brain --lib test_turn_telemetry_persistence_and_summary -- --nocapture

echo "==> agent-brain turn telemetry verification passed."
