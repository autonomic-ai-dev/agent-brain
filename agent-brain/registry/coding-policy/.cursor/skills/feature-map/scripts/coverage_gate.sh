#!/usr/bin/env sh
set -eu

if [ "$#" -lt 2 ]; then
  echo "usage: coverage_gate.sh <jacoco.xml> <min-line-percent> [min-branch-percent]" >&2
  exit 2
fi

XML="$1"
MIN_LINE="$2"
MIN_BRANCH="${3:-0}"

if [ ! -f "$XML" ]; then
  echo "FAIL no coverage report at $XML; run the unit tests with coverage first" >&2
  exit 1
fi

report=$(sed 's/<\/package>/\
/g' "$XML" | tail -n 1)

counter() {
  printf '%s\n' "$report" | grep -o "counter type=\"$1\" missed=\"[0-9]*\" covered=\"[0-9]*\"" | head -n 1 | sed 's/.*missed="\([0-9]*\)" covered="\([0-9]*\)".*/\1 \2/'
}

line=$(counter LINE)
branch=$(counter BRANCH)

if [ -z "$line" ]; then
  echo "FAIL $XML has no report-level LINE counter" >&2
  exit 1
fi

percent() {
  awk -v m="$1" -v c="$2" 'BEGIN { t = m + c; if (t == 0) print 100; else printf "%.2f", (c * 100) / t }'
}

line_pct=$(percent $line)
branch_pct=100
if [ -n "$branch" ]; then
  branch_pct=$(percent $branch)
fi

status=0
awk -v v="$line_pct" -v min="$MIN_LINE" 'BEGIN { exit !(v + 0 < min + 0) }' && status=1
awk -v v="$branch_pct" -v min="$MIN_BRANCH" 'BEGIN { exit !(v + 0 < min + 0) }' && status=1

if [ "$status" -eq 0 ]; then
  echo "ok line ${line_pct}% (floor ${MIN_LINE}%), branch ${branch_pct}% (floor ${MIN_BRANCH}%) from $XML"
else
  echo "FAIL line ${line_pct}% (floor ${MIN_LINE}%), branch ${branch_pct}% (floor ${MIN_BRANCH}%) from $XML" >&2
fi
exit "$status"
