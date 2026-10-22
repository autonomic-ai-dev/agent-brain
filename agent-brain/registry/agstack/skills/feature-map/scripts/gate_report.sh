#!/usr/bin/env sh
set -u

CONFIG="$1"
OUT="$2"
DIR=$(cd "$(dirname "$CONFIG")" && pwd)
HIGH_RISK="$DIR/high-risk.txt"
REVIEW="$OUT/review.txt"
REPORT="$OUT/evidence.md"
LOGS="$OUT/gates"

mkdir -p "$LOGS"
result=PASS
stopped=""
na=""
n=0

{
  printf '# Gate report\n\n'
  printf 'Config: `%s`\n\n' "$CONFIG"
  printf '| # | gate | status | exit | seconds | log |\n'
  printf '|---|---|---|---|---|---|\n'
} > "$REPORT"

while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    ''|'#'*) continue ;;
  esac
  name=${line%%|*}
  cmd=${line#*|}
  n=$((n + 1))

  case "$cmd" in
    NOT-AVAILABLE:*)
      reason=${cmd#NOT-AVAILABLE:}
      printf '| %s | %s | NOT AVAILABLE | - | - | %s |\n' "$n" "$name" "$reason" >> "$REPORT"
      na="$na
- $name: $reason"
      continue
      ;;
  esac

  if [ -n "$stopped" ]; then
    printf '| %s | %s | NOT RUN (stopped at %s) | - | - | - |\n' "$n" "$name" "$stopped" >> "$REPORT"
    continue
  fi

  log="$LOGS/$n-$name.log"
  start=$(date +%s)
  sh -c "$cmd" < /dev/null > "$log" 2>&1
  code=$?
  secs=$(( $(date +%s) - start ))
  if [ "$code" -eq 0 ]; then
    status=PASS
  else
    status=FAIL
    result=FAIL
    stopped=$name
  fi
  printf '| %s | %s | %s | %s | %s | %s |\n' "$n" "$name" "$status" "$code" "$secs" "$log" >> "$REPORT"
done < "$CONFIG"

changed=$(git status --porcelain -uall 2>/dev/null | sed 's/^...//' | sed 's/.* -> //')
matched=""
if [ -f "$HIGH_RISK" ]; then
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    while IFS= read -r pattern; do
      case "$pattern" in ''|'#'*) continue ;; esac
      case "$file" in
        $pattern) matched="$matched
- $file"; break ;;
      esac
    done < "$HIGH_RISK"
  done <<EOF
$changed
EOF
fi

review_status="not required (no high-risk change)"
if [ -n "$matched" ]; then
  author=""
  reviewer=""
  if [ -f "$REVIEW" ]; then
    author=$(sed -n 's/^author: *//p' "$REVIEW" | head -n 1)
    reviewer=$(sed -n 's/^reviewer: *//p' "$REVIEW" | head -n 1)
  fi
  owners="${CODEOWNERS_FILE:-CODEOWNERS}"
  missing=""
  for key in problem scope tests; do
    grep -q "^$key: *[^ ]" "$REVIEW" 2>/dev/null || missing="$missing $key"
  done
  if [ -z "$reviewer" ]; then
    review_status="REQUIRED - no reviewer recorded in $REVIEW"
    result=FAIL
  elif [ "$reviewer" = "$author" ]; then
    review_status="REQUIRED - reviewer must differ from author"
    result=FAIL
  elif [ -f "$owners" ] && ! grep -q -E "@${reviewer}([[:space:]]|\$)" "$owners"; then
    review_status="REQUIRED - reviewer $reviewer is not listed in $owners"
    result=FAIL
  elif [ -n "$missing" ]; then
    review_status="REQUIRED - review record lacks:$missing"
    result=FAIL
  else
    review_status="recorded (reviewer: $reviewer)"
  fi
fi

{
  printf '\n## Changed files\n'
  if [ -n "$changed" ]; then
    printf '%s\n' "$changed" | sed 's/^/- /'
  else
    printf 'none\n'
  fi
  printf '\n## High-risk files\n'
  if [ -n "$matched" ]; then
    printf '%s\n' "$matched"
  else
    printf 'none\n'
  fi
  printf '\n## Independent review\n%s\n' "$review_status"
  printf '\n## Gates not available\n'
  if [ -n "$na" ]; then
    printf '%s\n' "$na"
  else
    printf 'none\n'
  fi
  printf '\n## Result\n%s\n' "$result"
  if [ -n "$stopped" ]; then
    printf '\nStopped at gate `%s`. Fix its cause before continuing.\n' "$stopped"
  fi
  printf '\nThis record lists what was executed. It does not establish that the code is bug-free.\n'
} >> "$REPORT"

cat "$REPORT"
[ "$result" = PASS ]
