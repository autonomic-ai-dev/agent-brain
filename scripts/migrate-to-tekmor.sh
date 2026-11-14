#!/usr/bin/env bash
set -euo pipefail

DRY=0
if [ "${1:-}" = "--dry-run" ]; then DRY=1; fi
STAMP=$(date +%Y%m%d%H%M%S)
GITHUB_REPO="autonomic-ai-dev/tekmor"

say() { printf '%s\n' "$*"; }
act() {
  if [ "$DRY" = 1 ]; then say "would: $*"; else "$@"; fi
}

if ! command -v tekmor >/dev/null 2>&1; then
  say "tekmor not on PATH. Install first: scripts/install.sh. Then rerun."
  exit 1
fi

cd "$HOME"
shopt -s nullglob

if [ -d .agent_brain ] && [ ! -e .tekmor ]; then
  say "copy ~/.agent_brain to ~/.tekmor (original kept)"
  act cp -R .agent_brain .tekmor
fi

if [ -d .config/agent-spine ] && [ ! -e .config/thesmos ]; then
  say "copy ~/.config/agent-spine to ~/.config/thesmos (original kept)"
  act cp -R .config/agent-spine .config/thesmos
fi

rewrite_names_in() {
  local file="$1"
  grep -q -i -E 'agent[-_](brain|spine)|AGENT_BRAIN' "$file" || return 0
  say "rewrite names in ~/$file"
  if [ "$DRY" = 0 ]; then
    cp "$file" "$file.pre-tekmor-$STAMP"
    sed -i '' \
      -e "s#$GITHUB_REPO#@@REPO@@#g" \
      -e 's/AGENT_BRAIN/TEKMOR/g' \
      -e 's/agent_brain/tekmor/g' \
      -e 's/agent-brain/tekmor/g' \
      -e 's/agent-spine/thesmos/g' \
      -e "s#@@REPO@@#$GITHUB_REPO#g" \
      "$file"
  fi
}

rename_file() {
  local old="$1"
  local new="${old//agent-brain/tekmor}"
  say "rename ~/$old to ~/$new"
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
  .autonomic/memory/config.yaml
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
  say "old binary kept: ~/.local/bin/agent-brain. Remove after tekmor works."
fi

say "reinstall host hooks"
act tekmor install --global

say "restart host sessions and tekmor serve"
if [ "$DRY" = 0 ]; then
  say "check: tekmor doctor"
  tekmor doctor || say "doctor reported problems. See output above."
fi
