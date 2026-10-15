#!/usr/bin/env bash
set -euo pipefail
root="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "Run inside a Git repository." >&2; exit 2; }
script_dir="$(cd "$(dirname "$0")" && pwd)"
hooks_dir="$root/.git/hooks"
if [[ ! -d "$hooks_dir" ]]; then
  echo "Git hooks directory not found (worktree or custom core.hooksPath may be configured)." >&2
  echo "Install scripts/check-added-comments.sh through your repository's hook manager instead." >&2
  exit 2
fi
hook="$hooks_dir/pre-commit"
if [[ -e "$hook" ]]; then
  echo "Existing pre-commit hook found: $hook" >&2
  echo "Not overwriting it. Merge this check into your existing hook manually:" >&2
  echo '  "<path-to-bundle>/scripts/check-added-comments.sh" --staged' >&2
  exit 1
fi
cat > "$hook" <<HOOK
#!/usr/bin/env bash
set -euo pipefail
"$script_dir/check-added-comments.sh" --staged
HOOK
chmod +x "$hook"
echo "Installed comment guard: $hook"
echo "This blocks commits with detected newly added comments; it does not prevent editing or bypasses such as --no-verify."
