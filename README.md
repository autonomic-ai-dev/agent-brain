# agent-brain

Fast, local MCP server that routes each turn to the right **agents, skills, rules, and memory** under a strict token budget.

Rust is the brain; Cursor/Claude are the hands.

**MCP is live immediately** — `serve` starts stdio first; index sync, session ingest, and prewarm run in a background thread by default (`AGENT_BRAIN_BOOTSTRAP_BG=0` for the old blocking behavior).

## Why agent-brain?

Three problems every power-user hits with Cursor skills and rules:

1. **Context bloat** — hundreds of skills and rules cannot all fit in one turn. Stuffing them in degrades reasoning and burns tokens.
2. **Soft enforcement** — telling the model “use skills first” in a rule is optional. The agent can still grep, edit, or guess.
3. **No durable routing memory** — decisions from last week are not automatically surfaced as constraints on the next similar task.

**agent-brain** is a local **turn router**: one MCP call per user message returns a small, ranked bundle (agents, skills, rules, memory) inside a fixed token budget. Cursor hooks can hard-block other tools until that call succeeds.

## How it differs

| Approach | What it optimizes | agent-brain |
|----------|-------------------|-------------|
| **Cursor skills / rules (static)** | Authoring & discovery | Loads everything into context or hopes the model picks — no per-turn budget |
| **Memory SaaS** (Mem0, Zep, etc.) | Long-term chat / user memory | Cloud APIs, session recall — not skill/rule routing or editor enforcement |
| **Agent frameworks** (LangGraph, CrewAI, etc.) | Multi-step agent runtime | You still embed orchestration in app code — not a drop-in MCP gate for the IDE |
| **Vector / RAG stacks** (Chroma, Qdrant, etc.) | Document retrieval | General search — not opinionated about skills, phases, `must_apply`, or package scope |
| **agent-brain** | **Per-turn IDE routing** | Local BM25 + embeddings, SQLite memory, package installs, hook-enforced `route_task` |

**When to use it:** you already have (or want) a large skill library — ECC, superpowers, team rules — and need **fast, repeatable** context selection every turn, not another autonomous agent loop.

**When not to use it:** you only have a few project rules and the model already follows them; a full memory/RAG platform is enough for your use case.

## Lower latency

`route_task` is designed for the hot path (target under 50ms on warm cache). Today’s stack:

```text
user message → turn cache? → BM25 prefilter → (optional) embed → dot-score → token budget assembly
```

**Already on by default (0.3.x):**

| Mechanism | Effect |
|-----------|--------|
| Turn cache (60s LRU) | Repeat queries on the same message/scope return in ~0ms |
| BM25 fast path | Skips ONNX embed when FTS hits are strong |
| Bootstrap prewarm | Index + embedder warm before first real route |
| SQLite + in-RAM query embedding cache | Survives restarts; avoids re-embedding identical queries |
| `Arc` index snapshot + batched dot product | No full index clone per route; SIMD-friendly scoring |
| Skills/rules before memory in budget | Stops legacy session memories from crowding out skills |

**Tune with env vars** (in `~/.cursor/mcp.json` → `agent-brain` → `env`):

| Variable | Default | Set to disable / change |
|----------|---------|-------------------------|
| `AGENT_BRAIN_PREWARM` | on | `0` — skip startup prewarm |
| `AGENT_BRAIN_EMBEDDING_CACHE` | on | `0` — no persisted query vectors |
| `AGENT_BRAIN_BM25_FAST_PATH` | on | `0` — always embed |
| `AGENT_BRAIN_EMBED_MODEL` | `mini` | `fast`, `bge-small`, `bge-small-q` (trade quality vs speed) |
| `AGENT_BRAIN_TURN_CACHE_OPEN_FILES` | off | `1` — include open files in cache key (fewer hits) |
| `AGENT_BRAIN_SESSION_INGEST_BG` | on | `0` — block `serve` until session import finishes |
| `AGENT_BRAIN_BOOTSTRAP_BG` | on | `0` — block `serve` until index sync finishes (slower MCP enable) |
| `AGENT_BRAIN_BOOTSTRAP_DELAY_SEC` | `2` | Seconds before background bootstrap starts |
| `AGENT_BRAIN_BOOTSTRAP_INTERVAL_SEC` | `3600` | Skip bootstrap if indexed within this window |
| `AGENT_BRAIN_AUTO_UPDATE_DELAY_SEC` | `300` | Seconds after `serve` before auto-update runs |
| `AGENT_BRAIN_SESSION_INGEST_DELAY_SEC` | `180` | Extra delay before background session ingest |

**MCP restart after auto-update** is configured in `~/.agent_brain/config.yaml` (`mcp.restart_idle_secs`, `restart_max_wait_secs`, `restart_min_delay_secs`) — not env vars.

**Operational tips:** keep installed packages lean (`agent-brain package list`); use `RUST_LOG=agent_brain=info` and watch `latency_ms`, `cache_hit`, `bm25_fast_path`, `p95_ms` in stderr; pass explicit `limits` to `route_task` if your MCP client sends zeros; run `agent-brain version` to confirm the binary on disk.

## Storage direction (now vs next)

**Today (0.3.x):** single-user, local-first — no external DB required.

| Layer | Store | Role |
|-------|-------|------|
| Lexical | SQLite FTS5 | BM25 prefilter, cheap keyword gate |
| Semantic | ONNX embeddings in SQLite + in-memory unit vectors | Re-rank hundreds of candidates (not millions) |
| Memory / metadata | SQLite `facts` + indexed items | Durable decisions, `must_apply`, packages |
| Hot path | In-process `Arc` snapshot + turn LRU | Sub-ms reads after warm-up |

**When indexes grow (1k+ skills, cross-repo graphs), a mixed model makes sense:**

| Use case | Good fit | Why |
|----------|----------|-----|
| **Skill/rule retrieval at scale** | Local vector index (sqlite-vec, LanceDB, USearch) | ANN beats brute-force dot product when candidate sets stop shrinking enough after BM25 |
| **Package deps, “always apply with X”, team ownership** | Graph edges (SQLite adjacency now → Kùzu/Memgraph later) | Traversal and constraints — not primary semantic search |
| **Session / fact memory** | Stay in SQLite | Small, structured, transactional; sync-friendly |
| **Cold corpus (docs, tickets)** | Optional vector DB partition | Separate from per-turn skill routing — don’t merge with 500-token route budget |

**Pragmatic 0.4+ path:** keep BM25 + scope filter in SQLite → ANN rerank on the filtered set → graph only for dependency and `must_apply` expansion. Avoid routing every turn through a remote vector or graph service; latency belongs in-process.

**Full guide:** [docs/USAGE.md](docs/USAGE.md)

## Do I start the MCP manually?

**No.** After `agent-brain install --global`, **Cursor starts `agent-brain serve` automatically** when you open the editor. You only run `serve` yourself when debugging.

## Quick start (new laptop)

```bash
# 1. Install binary + write ~/.cursor/mcp.json
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/master/scripts/install.sh | bash -s -- --global

# 2. Restart Cursor, enable agent-brain under Settings → MCP

# 3. (Optional) Install ECC or other skill packages
agent-brain add affaan-m/ecc
```

That's it. Open Cursor in **Agent mode** — the editor starts agent-brain, indexes on boot, and the installed rule requires `route_task` each turn.

## What runs automatically

| Action | Who does it |
|--------|-------------|
| Start MCP server | Cursor spawns `agent-brain serve` |
| Index skills/rules/memory | agent-brain on MCP startup |
| Route each turn | Agent calls `route_task` — **blocked by Cursor hooks** until called |
| Persist decisions | Agent calls `store_memory` at task end |

`install --global` writes MCP config, a Cursor rule, and **hooks** (`~/.cursor/hooks.json`) that deny other tools until `route_task` runs each turn.

CLI is only for one-time setup (`install`, optional `add`) or maintenance (`package update`).

## Other agents

The MCP server is host-agnostic. **Cursor** has a one-command installer. **Claude Code / Codex / Claude Desktop** work with the same binary — add it to their MCP config manually; skills under `~/.claude/` and `~/.codex/` are already indexed. Host-specific installers come later.

## Install options

**Release binary** (from [Releases](https://github.com/aeswibon/agent-brain/releases)):

```bash
# download the binary for your OS, then:
chmod +x agent-brain-*
mv agent-brain-* ~/.local/bin/agent-brain
agent-brain install --global
```

**From source** (requires Rust + git):

```bash
cargo install --git https://github.com/aeswibon/agent-brain --locked agent-brain
agent-brain install --global
agent-brain add affaan-m/ecc   # optional packages
```

## Commands

| Command | Description |
|---------|-------------|
| `agent-brain install --global` | Write `~/.cursor/mcp.json` (one-time) |
| `agent-brain add <owner/repo>` | Install a GitHub skills/agents package |
| `agent-brain package list\|update\|remove` | Manage installed packages |
| `agent-brain index` | Force reindex (optional — also runs on MCP start) |
| `agent-brain version` | Print installed version |
| `agent-brain serve` | Manual MCP server (debug only) |

## MCP config

`agent-brain install --global` writes:

```json
{
  "mcpServers": {
    "agent-brain": {
      "command": "/Users/you/.local/bin/agent-brain",
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

Cursor spawns this process automatically — you do not need a terminal running `serve`.

## Packages

```bash
agent-brain add affaan-m/ecc
agent-brain package update ecc
```

Clones to `~/.agent_brain/packages/` and indexes skills, agents, rules, and commands.

## Data directory

`~/.agent_brain/` (override with `AGENT_BRAIN_HOME`). First MCP start downloads the embedding model (~90MB).

## Development

```bash
cargo test --release -p agent-brain
make release-macos             # macOS: build + adhoc sign (required for Cursor MCP)
cargo build --release -p agent-brain
```

On macOS, release CI artifacts and `install.sh` adhoc-sign binaries; local builds need `make release-macos` or `agent-brain doctor --fix`.

## Releases

See [CHANGELOG.md](CHANGELOG.md). Tags `v*` publish platform binaries with changelog-based release notes.

## License

MIT
