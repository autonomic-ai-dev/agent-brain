# agent-brain

Fast, local MCP server that routes each turn to the right agents, skills, rules, and memory under a strict token budget.

**Setup guide:** [../docs/USAGE.md](../docs/USAGE.md)

## Why agent-brain?

While there are many AI agent frameworks and memory tools available, **agent-brain** is specifically designed to act as a fast, local traffic cop between the AI and your codebase. It differentiates itself through highly opinionated architectural choices:

- **Hard-Gated Action Enforcement**: Rather than relying on soft system prompts, agent-brain can install editor hooks that physically block the AI from using destructive or exploratory tools until it successfully retrieves necessary context.
- **Strict Token Budgeting**: Instead of blindly stuffing all available `.cursorrules` and skills into the context window—which degrades AI reasoning—agent-brain performs local semantic routing to fetch only the exact skills, agents, and rules needed for the current turn.
- **Universal Package Management**: It functions as a package manager for AI behaviors. You can install remote GitHub repositories containing collections of skills and agents with a single command (e.g., `agent-brain add affaan-m/ecc`).
- **Durable Memory & Constraints**: It utilizes a local SQLite database to persist facts and hard constraints (`must_apply`) across sessions, preventing the AI from repeating the same mistakes.
- **Local & Sub-50ms Routing**: Built in Rust and utilizing a local embedding model, the routing step takes less than 50ms on repeat queries, keeping the interaction feeling instantaneous without relying on external APIs.

## MCP auto-start (important)

You do **not** run `agent-brain serve` in a terminal for normal Cursor use.

1. Run `agent-brain install --global` once
2. Restart Cursor
3. Cursor automatically spawns `agent-brain serve` when MCP is needed

Use `agent-brain serve` only for debugging MCP outside Cursor.

## Quick setup

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/main/scripts/install.sh | bash -s -- --global
# restart Cursor → Settings → MCP → enable agent-brain
agent-brain add affaan-m/ecc   # optional
```

## Install packages (ECC, etc.)

Add a GitHub repo of skills/agents/rules in one command:

```bash
agent-brain add affaan-m/ecc
# or
agent-brain add https://github.com/affaan-m/ecc
```

This shallow-clones into `~/.agent_brain/packages/ecc/` and indexes:

- `skills/`, `agents/`, `rules/`, `commands/`
- `.cursor/rules`, `.claude/skills`, and other standard paths
- Optional `agent-brain.yaml` manifest for custom roots

Manage packages:

```bash
agent-brain package list
agent-brain package update ecc
agent-brain package remove ecc
```

## Install MCP server

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
# Debug MCP manually (not needed for Cursor)
cargo run --release -p agent-brain -- serve

# Reindex on demand
cargo run --release -p agent-brain -- index
```

Logs go to **stderr** only — stdout is reserved for MCP JSON-RPC.

## Phase 1 features

- Indexes agents, skills, rules, and memory from local paths and packages
- MCP tools: `route_task`, `get_context`, `store_memory`, `list_memory`, `delete_memory`, `export_memory`
- Turn cache (LRU, 60s TTL) for sub-50ms repeat queries
- Write queue with SQLite WAL for durable memory writes

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
