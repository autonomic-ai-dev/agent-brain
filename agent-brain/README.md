# agent-brain

Fast, local MCP server that routes each turn to the right agents, skills, rules, and memory under a strict token budget.

## Phase 1 (v0.3)

- Indexes agents, skills, rules, and memory from local paths
- MCP tools: `route_task`, `get_context`, `store_memory`, `list_memory`, `delete_memory`, `export_memory`
- Turn cache (LRU, 60s TTL) for sub-50ms repeat queries
- Write queue with SQLite WAL for durable memory writes

## Install

### One-liner (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/main/scripts/install.sh | bash -s -- --global
```

### From source

```bash
cargo install --git https://github.com/aeswibon/agent-brain --locked agent-brain
agent-brain install --global
```

### Configure Cursor manually

Run `agent-brain install --global` after building, or add to `~/.cursor/mcp.json`:

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

## Build

```bash
cargo build --release -p agent-brain
```

First run downloads the `AllMiniLML6V2` embedding model via fastembed (~90MB).

## Run

```bash
# Start MCP server (stdio)
cargo run --release -p agent-brain -- serve

# One-shot reindex
cargo run --release -p agent-brain -- index
```

Logs go to **stderr** only — stdout is reserved for MCP JSON-RPC.

## Cursor MCP config

Add to `.cursor/mcp.json` (or global MCP settings):

```json
{
  "mcpServers": {
    "agent-brain": {
      "command": "/path/to/mcp/target/release/agent-brain",
      "args": ["serve"],
      "env": {
        "RUST_LOG": "agent_brain=info"
      }
    }
  }
}
```

Replace the command path with your built binary. Override data directory with `AGENT_BRAIN_HOME` (default: `~/.agent_brain/`).

## Primary tool

Call **`route_task`** every turn with the user's message. It returns:

- `recommended_agents`, `recommended_skills`, `applicable_rules`, `relevant_memory`
- `must_apply` — hard constraints from memory
- `recommended_phase`, `tokens_used`, `cache_hit`, `latency_ms`

## Indexed paths

| Source | Path |
|--------|------|
| Brain home rules/skills/agents | `~/.agent_brain/{rules,skills,agents}/` |
| Cursor skills | `~/.cursor/skills/`, `~/.cursor/skills-cursor/` |
| Claude agents/skills | `~/.claude/{agents,skills}/` |
| Codex | `~/.codex/{agents,skills}/` |
| Repo rules | `.cursor/rules/`, `CLAUDE.md`, `AGENTS.md` |

## Tests

```bash
cargo test --release -p agent-brain
```
