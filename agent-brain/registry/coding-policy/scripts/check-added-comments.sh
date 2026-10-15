#!/usr/bin/env bash
set -euo pipefail

mode="working"
base_ref=""
while (($#)); do
  case "$1" in
    --staged) mode="staged"; shift ;;
    --base)
      [[ $# -ge 2 ]] || { echo "ERROR: --base requires a Git ref." >&2; exit 2; }
      mode="base"; base_ref="$2"; shift 2 ;;
    *) echo "Usage: $0 [--staged | --base REF]" >&2; exit 2 ;;
  esac
done
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "ERROR: run this check inside a Git repository." >&2
  exit 2
fi
case "$mode" in
  staged) diff_cmd=(git diff --cached --no-ext-diff --unified=0 --) ;;
  base)
    git rev-parse --verify "$base_ref^{commit}" >/dev/null 2>&1 || { echo "ERROR: base ref '$base_ref' is not available." >&2; exit 2; }
    diff_cmd=(git diff "$base_ref...HEAD" --no-ext-diff --unified=0 --)
    ;;
  *) diff_cmd=(git diff HEAD --no-ext-diff --unified=0 --) ;;
esac
output="$("${diff_cmd[@]}" || true)"
current_file=""
violations=0
while IFS= read -r line; do
  if [[ "$line" == '+++ b/'* ]]; then
    current_file="${line#+++ b/}"
    continue
  fi
  [[ "$line" == '+'* && "$line" != '+++'* ]] || continue
  added="${line:1}"
  ext="${current_file##*.}"
  case "$ext" in
    md|mdx|rst|txt|adoc|lock|sum|svg|jsonl) continue ;;
  esac
  [[ "$added" =~ ^[[:space:]]*\#! ]] && continue
  is_comment=0
  case "$ext" in
    js|jsx|ts|tsx|java|kt|kts|go|rs|c|h|cc|cpp|hpp|cs|swift|scala|php|dart|css|scss|less)
      [[ "$added" =~ ^[[:space:]]*// ]] && is_comment=1
      [[ "$added" =~ ^[[:space:]]*/\* ]] && is_comment=1
      [[ "$added" =~ ^[[:space:]]*\*([[:space:]]|/) ]] && is_comment=1
      [[ "$added" =~ [[:space:]]//[[:space:]] ]] && is_comment=1
      [[ "$added" =~ /\*.*\*/ ]] && is_comment=1
      ;;
    py|pyi|sh|bash|zsh|yaml|yml|toml|ini|cfg|conf|env|Dockerfile)
      [[ "$added" =~ ^[[:space:]]*# ]] && is_comment=1
      ;;
    sql|lua|hs|elm)
      [[ "$added" =~ ^[[:space:]]*-- ]] && is_comment=1
      ;;
    html|htm|xml|xhtml|vue|svelte)
      [[ "$added" =~ '<!--' ]] && is_comment=1
      ;;
    rb|rake|pl|pm|make|mk|cmake)
      [[ "$added" =~ ^[[:space:]]*# ]] && is_comment=1
      ;;
    *)
      [[ "$added" =~ ^[[:space:]]*(//|/\*|<!--|--) ]] && is_comment=1
      ;;
  esac
  if (( is_comment )); then
    printf 'Possible added comment: %s\n  %s\n' "$current_file" "$added" >&2
    violations=$((violations + 1))
  fi
done <<< "$output"
if (( violations > 0 )); then
  echo >&2
  echo "BLOCKED: found $violations added comment-like line(s). Remove them unless the user explicitly requested comments, then rerun." >&2
  exit 1
fi
echo "PASS: no added comment-like lines detected in the selected Git diff."
