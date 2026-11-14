# Changelog

The release history starts at v1.0.0-rc1. Earlier history is in git.

## v1.0.0-rc1 (not yet tagged)

tekmor is a proof eval for coding agents.

### Added

- `tekmor verify -- <cmd>` runs an allowlisted command from `.verify/verify.toml` and records its outcome with the code hash and commit.
- `tekmor verify receipt` reports whether the current commit and code passed. It exits 0 only when it passed. `--markdown` and `--json` are available.
- `tekmor verify summary` shows the checks that fail most.
- `tekmor verify init` creates `.verify/` with an allowlist.
- `tekmor stop-gate` blocks a turn that changed code without a passing run or a feature map. It runs as the Claude Code Stop hook.
- `tekmor bash-gate` blocks or warns on risky shell commands and refuses commits that skip git hooks. It runs as the Claude Code PreToolUse hook for Bash.
- `tekmor install [--global]` registers the two gates in Claude Code settings.
- Feature maps in `.verify/features/` tie code to features and to the checks that prove them.
- `tekmor-core`: pure evidence, gate rule and verify types with no storage, runtime or IO dependencies.
- `tekmor ext list|add|remove` manages optional extensions. None are bundled.

### Changed

- The verify store keeps only verify outcomes and metadata. The schema starts at version 29; older verify tables are upgraded in place.

### Removed

- The MCP server, daemon, proxy, route tool and route gate.
- Retrieval, memory, lessons, trajectories, packages, upstream MCP servers, sync, Memorable, graphify, dashboard, stats, token tools and suggest-memory.
