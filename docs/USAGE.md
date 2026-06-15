# agent-brain usage guide

## Do I need to start the MCP manually?

**No — not for normal Cursor use.**

Once `agent-brain` is in your Cursor MCP config (`~/.cursor/mcp.json`), **Cursor starts it automatically** when you open the editor or when an agent needs MCP tools. You do not need a separate terminal running `agent-brain serve`.

| Scenario | What to run |
|----------|-------------|
| Daily Cursor usage | Nothing — Cursor spawns `agent-brain serve` for you |
| First-time setup | `agent-brain install --global` once |
| Add skills package | `agent-brain add affaan-m/ecc` once |
| Refresh index after changes | `agent-brain index` (optional; `serve` also indexes on startup) |
| Debug MCP protocol | `agent-brain serve` manually in a terminal |

After editing `mcp.json`, **restart Cursor** (or reload MCP in **Settings → MCP**) so it picks up the config.

## First-time setup (new laptop)

### 1. Install the binary

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/main/scripts/install.sh | bash -s -- --global
```

Or from source:

```bash
cargo install --git https://github.com/aeswibon/agent-brain --locked agent-brain
agent-brain install --global
```

### 2. Register with Cursor

`agent-brain install --global` writes `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "agent-brain": {
      "command": "/Users/you/.local/bin/agent-brain",
      "args": ["serve"],
      "env": { "RUST_LOG": "agent_brain=info" }
    }
  }
}
```

Restart Cursor.

### 3. Enable the server

1. Open **Cursor Settings → MCP**
2. Confirm `agent-brain` appears
3. Toggle it **on** if prompted

### 4. Install skill packages (optional)

```bash
agent-brain add affaan-m/ecc
```

This clones ECC into `~/.agent_brain/packages/ecc/` and indexes it.

### 5. Verify

In Cursor Agent chat, the model should have access to tools like `route_task`. Ask:

> Use route_task for: "fix a failing rust test"

Check **Settings → MCP** — the server status should be green after the first connection.

## Daily workflow

1. Open Cursor — MCP starts in the background automatically
2. Each agent turn can call **`route_task`** with the user message
3. agent-brain returns recommended agents, skills, rules, and memory under a token budget
4. The agent uses those recommendations to pick tools/skills

You only need `agent-brain index` if you:

- Added new local skills/rules under `~/.agent_brain/`
- Installed or updated a package (`agent-brain package update`)
- Want to force a reindex without restarting Cursor

## Package commands

```bash
# Install Everything Claude Code (ECC)
agent-brain add affaan-m/ecc
agent-brain add https://github.com/affaan-m/ecc

# List installed packages
agent-brain package list

# Update to latest
agent-brain package update ecc

# Remove
agent-brain package remove ecc
```

Packages live at `~/.agent_brain/packages/<name>/`.

## MCP tools reference

| Tool | When to use |
|------|-------------|
| `route_task` | **Every turn** — primary routing with token budget |
| `get_context` | Lower-level flat context retrieval |
| `store_memory` | Persist a short fact at task end (max 50 words) |
| `list_memory` | Inspect stored facts |
| `delete_memory` | Remove a fact |
| `export_memory` | Export facts to `~/.agent_brain/export/` |

## Troubleshooting

### MCP server shows red / failed to start

1. Confirm binary exists: `which agent-brain`
2. Use an **absolute path** in `mcp.json` (run `agent-brain install --global` again)
3. Check logs: **View → Output → MCP** (select `agent-brain`)
4. First start downloads the embedding model (~90MB) — can take 1–2 minutes

### `route_task` not appearing

- Ensure Agent mode (not Ask-only) in chat
- Confirm MCP server is enabled in settings
- Restart Cursor after config changes

### Slow first query

- First run downloads `AllMiniLML6V2` and builds the index
- Run `agent-brain index` once to warm the index before using Cursor

### Package install fails

- Requires `git` on PATH
- Check network access to GitHub
- Try explicit ref: `agent-brain add affaan-m/ecc --ref main`

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `AGENT_BRAIN_HOME` | `~/.agent_brain` | Data, packages, database |
| `AGENT_BRAIN_SESSION_INGEST` | `1` (on) | Set `0` or `false` to disable legacy session import |
| `RUST_LOG` | `agent_brain=info` | Log level (stderr only) |

## Legacy session import (0.3.2 hack)

On startup, agent-brain scans recent Cursor (`~/.cursor/projects/**/agent-transcripts/`) and Codex (`~/.codex/sessions/`) JSONL files and imports user messages into memory. This is a temporary bridge until proper session digests ship in 0.3.4.

- Skips files already ingested (content hash in `brain.db` meta)
- Caps at 150 files and 12 user messages per file per run
- Disable with `AGENT_BRAIN_SESSION_INGEST=0`
