# Host integration guide

agent-brain is a **stdio MCP server**. Any editor or CLI that supports MCP can use the same binary — only the config file location and JSON shape differ.

## Quick install

```bash
# Cursor (global + hooks + project rule)
agent-brain install --global

# Claude Desktop
agent-brain install --claude-desktop

# VS Code — workspace `.vscode/mcp.json`
agent-brain install --vscode

# VS Code — user profile (all workspaces)
agent-brain install --vscode --global

# Claude Code — project `.mcp.json`
agent-brain install --claude-code

# Claude Code — all projects (`~/.claude.json`)
agent-brain install --claude-code --global

# Everything at once
agent-brain install --all
```

Print JSON without writing files:

```bash
agent-brain install --print-only
agent-brain install --vscode --print-only
```

## Config file locations

| Host | Scope | File |
|------|-------|------|
| **Cursor** | global | `~/.cursor/mcp.json` |
| **Cursor** | workspace | `.cursor/mcp.json` |
| **Claude Desktop** | user | macOS: `~/Library/Application Support/Claude/claude_desktop_config.json` |
| **VS Code** | workspace | `.vscode/mcp.json` |
| **VS Code** | user | macOS: `~/Library/Application Support/Code/User/mcp.json` |
| **Claude Code** | project | `.mcp.json` (repository root) |
| **Claude Code** | user | `~/.claude.json` |

### JSON shape

**Cursor / Claude Desktop / Claude Code** use `mcpServers`:

```json
{
  "mcpServers": {
    "agent-brain": {
      "command": "/absolute/path/to/agent-brain",
      "args": ["serve"],
      "env": { "RUST_LOG": "agent_brain=info" }
    }
  }
}
```

**VS Code** uses `servers` with `type: "stdio"`:

```json
{
  "servers": {
    "agent-brain": {
      "type": "stdio",
      "command": "/absolute/path/to/agent-brain",
      "args": ["serve"]
    }
  }
}
```

**Claude Code** project/user entries also include `"type": "stdio"`.

## Required agent workflow

Every host should call these MCP tools:

1. **`route_task`** — start of each user turn (before planning or edits)
2. **`store_memory`** — end of task when a durable decision was made (max 50 words)

Other tools (`get_context`, `route_to_mcp`, `import_memory`, …) are optional.

Readable route summary: `~/.agent_brain/logs/last-route.md` or `agent-brain briefing`.

## Hook parity matrix

| Host | `route_task` gate | How enforced |
|------|-------------------|--------------|
| **Cursor** | Yes | `~/.cursor/hooks.json` → `route_gate.py` (installed by `install --global`) |
| **Claude Code** | Rule template | `~/.claude/agent-brain.md` or `.claude/agent-brain.md` (installed by `install --claude-code`) |
| **VS Code** | Rule-only | Add Copilot/custom instructions (see below) |
| **Claude Desktop** | Rule-only | Paste rule into project/system instructions |
| **Windsurf / Zed / other** | Manual | Copy `mcpServers` block; add equivalent rule text |

Cursor is the only host with **automatic pre-tool hooks** today. Other hosts rely on rules/instructions plus operator discipline.

### VS Code instructions snippet

Add to `.github/copilot-instructions.md` or your user Copilot instructions:

```markdown
## agent-brain MCP

At the start of every user turn, call the `route_task` MCP tool on server `agent-brain`
with `user_message`, `current_working_directory`, and `open_files`. Apply returned
skills, rules, and memory. At task end, call `store_memory` for durable outcomes.
```

## Claude Code gotchas

- **Do not** put `mcpServers` in `~/.claude/settings.json` — Claude Code **silently ignores** it.
- User-scoped servers belong in **`~/.claude.json`** (top-level `mcpServers`).
- Project-scoped servers belong in **`.mcp.json`** at the repo root (not inside `.claude/`).
- After editing, start a new session and run `/mcp` to verify connection.

## macOS codesign

Cursor launches MCP via taskgated. Linker-signed local builds may be killed.

```bash
make release-macos          # or download GitHub release binary
agent-brain doctor --fix    # adhoc re-sign + align mcp.json
```

## Index paths (already wired)

agent-brain indexes skills/agents/rules from:

- `~/.cursor/`, `~/.claude/`, `~/.codex/`
- `~/.agent_brain/packages/*`
- Project `.cursor/rules`, `.claude/agents`, etc.

No extra index configuration is required per host — installers only register the MCP server.

## Verification checklist

| Host | Verify |
|------|--------|
| Cursor | Settings → MCP shows agent-brain; hooks enabled; `route_task` in MCP log |
| Claude Desktop | Hammer icon; Developer → MCP logs |
| VS Code | Command Palette → “MCP: List Servers” → agent-brain connected |
| Claude Code | `/mcp` lists agent-brain; tools appear after approval |
