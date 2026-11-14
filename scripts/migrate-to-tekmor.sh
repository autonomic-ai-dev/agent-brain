#!/usr/bin/env bash
set -euo pipefail

DRY=0
if [ "${1:-}" = "--dry-run" ]; then DRY=1; fi
STAMP=$(date +%Y%m%d%H%M%S)
GITHUB_REPO="autonomic-ai-dev/agent-brain"

say() { printf '%s\n' "$*"; }
act() {
  if [ "$DRY" = 1 ]; then say "would: $*"; else "$@"; fi
}

if ! command -v tekmor >/dev/null 2>&1; then
  say "tekmor is not on PATH. Install it first (scripts/install.sh), then rerun this script."
  exit 1
fi

cd "$HOME"
shopt -s nullglob

if [ -d .agent_brain ] && [ ! -e .tekmor ]; then
  say "copy legacy data home ~/.agent_brain -> ~/.tekmor (original kept)"
  act cp -R .agent_brain .tekmor
fi

rewrite_names_in() {
  local file="$1"
  grep -q -i -E 'agent[-_]brain|AGENT_BRAIN' "$file" || return 0
  say "rewrite names in ~/$file"
  if [ "$DRY" = 0 ]; then
    cp "$file" "$file.pre-tekmor-$STAMP"
    sed -i '' \
      -e "s#$GITHUB_REPO#@@REPO@@#g" \
      -e 's/AGENT_BRAIN/TEKMOR/g' \
      -e 's/agent_brain/tekmor/g' \
      -e 's/agent-brain/tekmor/g' \
      -e "s#@@REPO@@#$GITHUB_REPO#g" \
      "$file"
  fi
}

rename_file() {
  local old="$1"
  local new="${old//agent-brain/tekmor}"
  say "rename ~/$old -> ~/$new"
  act mv "$old" "$new"
}

HOST_FILES=(
  .claude.json
  .claude/settings.json
  .claude/AGENTS.md
  .cursor/hooks.json
  .codex/hooks.json
  .codex/config.toml
  .codex/AGENTS.md
  .gemini/settings.json
  .gemini/AGENTS.md
  .gemini/antigravity/AGENTS.md
  .gemini/antigravity/mcp_config.json
  .gemini/config/mcp_config.json
  .config/opencode/AGENTS.md
  .tekmor/config.yaml
  .zshrc
  .zprofile
  .bashrc
  .profile
)

for rule in .claude/rules/agent-brain-*.md .cursor/rules/agent-brain*.mdc .gemini/antigravity/rules/agent-brain-*.md; do
  rename_file "$rule"
done

for file in "${HOST_FILES[@]}"; do
  if [ -f "$file" ]; then rewrite_names_in "$file"; fi
done

if [ -e .local/bin/agent-brain ]; then
  say "old binary ~/.local/bin/agent-brain is kept; remove it once tekmor works"
fi

say "reinstall host hooks with the new binary"
act tekmor install --global

say "restart running host sessions and any tekmor serve process"
if [ "$DRY" = 0 ]; then
  say "check: tekmor doctor"
  tekmor doctor || say "doctor reported problems; read its output above"
fi
