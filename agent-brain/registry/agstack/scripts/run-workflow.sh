#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PYTHON_EXEC="python3"

if ! command -v "$PYTHON_EXEC" >/dev/null 2>&1; then
  echo "Error: python3 is required for run-workflow.sh" >&2
  exit 1
fi

exec "$PYTHON_EXEC" "$SCRIPT_DIR/run-workflow.py" "$@"
