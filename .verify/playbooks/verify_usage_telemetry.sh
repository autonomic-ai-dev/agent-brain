#!/usr/bin/env bash
set -euo pipefail

echo "==> Running tekmor turn telemetry and pricing tests..."
cargo test -p tekmor --lib telemetry::tests -- --nocapture

echo "==> Running tekmor turn telemetry store persistence tests..."
cargo test -p tekmor --lib test_turn_telemetry_persistence_and_summary -- --nocapture

echo "==> tekmor turn telemetry verification passed."
