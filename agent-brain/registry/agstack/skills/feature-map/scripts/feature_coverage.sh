#!/usr/bin/env sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: feature_coverage.sh <feature.md> <functional.hurl>" >&2
  exit 2
fi

FEATURE_MD="$1"
HURL="$2"

entries=$(grep -c -E '^(GET|POST|PUT|PATCH|DELETE) ' "$HURL" || true)
if [ "$entries" -eq 0 ]; then
  echo "FAIL $HURL has no request entries" >&2
  exit 1
fi

steps=$(grep -c -E '^[0-9]+\. \*\*' "$FEATURE_MD" || true)
annotated=$(grep -E '^[0-9]+\. \*\*' "$FEATURE_MD" | grep -c '(entries ' || true)
if [ "$steps" -ne "$annotated" ]; then
  echo "FAIL $FEATURE_MD: $((steps - annotated)) step(s) have no (entries ...) annotation" >&2
  grep -E '^[0-9]+\. \*\*' "$FEATURE_MD" | grep -v '(entries ' >&2 || true
  exit 1
fi

ranges=$(grep -E '^[0-9]+\. \*\*' "$FEATURE_MD" | sed -n 's/.*(entries \([^)]*\)).*/\1/p' | sed 's/–/-/g')

printf '%s\n' "$ranges" | awk -v n="$entries" -v file="$FEATURE_MD" -F',' '
{
  for (t = 1; t <= NF; t++) {
    tok = $t
    gsub(/ /, "", tok)
    if (tok == "") continue
    k = split(tok, p, "-")
    if (k == 2) {
      for (i = p[1] + 0; i <= p[2] + 0; i++) cover[i]++
    } else {
      cover[tok + 0]++
    }
  }
}
END {
  bad = 0
  for (i = 1; i <= n; i++) {
    if (cover[i] == 0) { print "FAIL " file ": entry " i " is covered by no step"; bad = 1 }
    else if (cover[i] > 1) { print "FAIL " file ": entry " i " is covered by " cover[i] " steps"; bad = 1 }
  }
  for (i in cover) {
    if (i + 0 < 1 || i + 0 > n) { print "FAIL " file ": a step refers to entry " i ", outside 1.." n; bad = 1 }
  }
  if (bad) exit 1
  print "ok " file ": " n " entries covered exactly once"
}'
