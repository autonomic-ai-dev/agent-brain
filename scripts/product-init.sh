#!/usr/bin/env bash
# Scaffold CONSTITUTION.md, docs/features/, and product-layer cursor rule in a target repo.
set -euo pipefail

DIR="."
FEATURE=""
BRAIN_ROOT=""

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \?//'
  exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --feature) FEATURE="$2"; shift 2 ;;
    --brain-root) BRAIN_ROOT="$2"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) echo "unknown arg: $1" >&2; usage 1 ;;
  esac
done

if [[ -z "$BRAIN_ROOT" ]]; then
  if command -v agent-brain >/dev/null 2>&1; then
    BRAIN_ROOT="$(dirname "$(dirname "$(command -v agent-brain)")")"
    # agent-brain binary may be in ~/.local/bin — fall back to sibling checkout
    if [[ ! -f "$BRAIN_ROOT/docs/product/CONSTITUTION.template.md" ]]; then
      BRAIN_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
    fi
  else
    BRAIN_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
  fi
fi

TEMPL="$BRAIN_ROOT/docs/product"
if [[ ! -d "$TEMPL" ]]; then
  echo "templates not found at $TEMPL" >&2
  exit 1
fi

mkdir -p "$DIR/docs/features" "$DIR/.cursor/rules"

if [[ ! -f "$DIR/CONSTITUTION.md" ]]; then
  cp "$TEMPL/CONSTITUTION.template.md" "$DIR/CONSTITUTION.md"
  echo "created CONSTITUTION.md"
else
  echo "CONSTITUTION.md exists — skipped"
fi

if [[ ! -f "$DIR/.cursor/rules/product-layer.mdc" ]]; then
  cp "$BRAIN_ROOT/agent-brain/registry/product-layer/.cursor/rules/product-layer.mdc" "$DIR/.cursor/rules/product-layer.mdc" 2>/dev/null || \
  cat > "$DIR/.cursor/rules/product-layer.mdc" <<'RULEEOF'
---
description: Product layer — constitution and feature spec drive implementation
alwaysApply: true
---
# Product layer
Read CONSTITUTION.md and active docs/features/*.md before implementing.
RULEEOF
  echo "created .cursor/rules/product-layer.mdc"
fi

if [[ -n "$FEATURE" ]]; then
  dest="$DIR/docs/features/${FEATURE}.md"
  if [[ ! -f "$dest" ]]; then
    cp "$TEMPL/FEATURE-SPEC.template.md" "$dest"
    sed -i '' "s/<title>/${FEATURE}/g" "$dest" 2>/dev/null || sed -i "s/<title>/${FEATURE}/g" "$dest"
    echo "created $dest"
  else
    echo "$dest exists — skipped"
  fi
fi

echo "Done. Edit CONSTITUTION.md and docs/features/*.md then run:"
echo "  agent-spine init --with feature-from-spec"
