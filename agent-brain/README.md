# agent-brain

Fast, local MCP server that routes each turn to the right agents, skills, rules, and memory under a strict token budget.

**Setup guide:** [../docs/USAGE.md](../docs/USAGE.md)

## Why agent-brain?

See the [repository README](../README.md#why-agent-brain) for positioning, comparisons, latency tuning, and storage direction.

In short: **per-turn routing** under a token budget, **hook-enforced** `route_task`, and **local** skill packages + memory — not a cloud memory SaaS or agent framework.

## MCP auto-start (important)

You do **not** run `agent-brain serve` in a terminal for normal Cursor use.

1. Run `agent-brain install --global` once
2. Restart Cursor
3. Cursor automatically spawns `agent-brain serve` when MCP is needed

Use `agent-brain serve` only for debugging MCP outside Cursor.

## Quick setup

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/master/scripts/install.sh | bash -s -- --global
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

### Auto-update (packages + MCP)

Create `~/.agent_brain/config.yaml` (or run `agent-brain config init`):

```yaml
auto_update:
  enabled: true
  interval_hours: 24
  packages:
    enabled: true
  mcp:
    enabled: true
    repo: aeswibon/agent-brain
    bin_path: ~/.local/bin/agent-brain
    refresh_cursor: true
    restart_after_update: true
    restart_idle_secs: 10
    restart_max_wait_secs: 300
    restart_min_delay_secs: 2
```

When enabled, MCP `serve` checks for updates in the background (packages via `git fetch`, MCP via GitHub releases). After an MCP binary update during `serve`, the process **restarts itself** when idle (Unix: `exec serve`) so Cursor reconnects without a full IDE restart. Tune with `restart_idle_secs`, `restart_max_wait_secs`, and `restart_min_delay_secs`. Disable with `mcp.restart_after_update: false`.

Run manually with `agent-brain update --force` (CLI updates disk only; toggle MCP in Settings if `serve` is already running).

Environment overrides: `AGENT_BRAIN_AUTO_UPDATE`, `AGENT_BRAIN_AUTO_UPDATE_PACKAGES`, `AGENT_BRAIN_AUTO_UPDATE_MCP`, `AGENT_BRAIN_AUTO_UPDATE_INTERVAL_HOURS`, `AGENT_BRAIN_AUTO_UPDATE_DELAY_SEC`.

## Install MCP server

### One-liner (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/master/scripts/install.sh | bash -s -- --global
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
      "env": {
        "RUST_LOG": "agent_brain=warn",
        "AGENT_BRAIN_BOOTSTRAP_BG": "1",
        "AGENT_BRAIN_BOOTSTRAP_DELAY_SEC": "2",
        "AGENT_BRAIN_BOOTSTRAP_INTERVAL_SEC": "3600",
        "AGENT_BRAIN_AUTO_UPDATE_DELAY_SEC": "300",
        "AGENT_BRAIN_SESSION_INGEST_DELAY_SEC": "180"
      }
    }
  }
}
```

## Build

On macOS, use `make release-macos` (or `./scripts/build-release-macos.sh`) to build and adhoc-sign so Cursor MCP can launch the binary. Plain `cargo build --release` produces a linker-signed binary that taskgated kills until you run `agent-brain doctor --fix` or `./scripts/sign-macos.sh`.

```bash
make release-macos             # macOS: build + adhoc sign
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
