#!/usr/bin/env bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "==> [tekmor] Starting verification suite..."

echo "==> cargo check"
cargo check -p tekmor

echo "==> tekmor-core boundary"
.verify/checks/core_boundary.sh

echo "==> tekmor-core unit tests"
cargo test -p tekmor-core

echo "==> tekmor tests"
cargo test -p tekmor -- --test-threads=1

echo "==> [tekmor] Verification completed successfully (exit 0)."
