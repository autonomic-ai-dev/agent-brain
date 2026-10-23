#!/bin/sh
set -eu

run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
log="$(dirname "$0")/../artifacts/require-tests/$run_id.log"
mkdir -p "$(dirname "$log")"

status=0
cargo test "$@" >"$log" 2>&1 || status=$?
cat "$log"
passed=$(awk '/^test result:/ {n += $4} END {print n + 0}' "$log")
echo "passed=$passed" >>"$log"
if [ "$status" -ne 0 ]; then
  exit "$status"
fi

if [ "$passed" -eq 0 ]; then
  echo "require_tests: no tests matched the filter: cargo test $*" >&2
  exit 1
fi
echo "require_tests: $passed passed"
