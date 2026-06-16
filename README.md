# agent-brain

Fast, **local** MCP server that routes each turn to the right **agents, skills, rules, and memory** under a strict token budget — with **hook enforcement** so the agent actually uses them.

Rust is the brain; Cursor/Claude are the hands.

**MCP is live immediately** — `serve` starts stdio first; index sync, session ingest, and prewarm run in a background thread by default.

---

## Why agent-brain?

Three problems every power-user hits with Cursor skills and rules:

1. **Context bloat** — hundreds of skills and rules cannot all fit in one turn. Stuffing them in degrades reasoning and burns tokens.
2. **Soft enforcement** — telling the model “use skills first” in a rule is optional. The agent can still grep, edit, or guess.
3. **No durable routing memory** — decisions from last week are not automatically surfaced as constraints on the next similar task.

**agent-brain fixes this with a local turn router:**

| Problem | agent-brain answer |
|---------|-------------------|
| Too much to load | **`route_task`** returns ~500 tokens of the *right* skills/rules/memory for *this* message |
| Model skips skills | **Cursor hooks** block other tools until `route_task` succeeds each turn |
| Forgotten conventions | **`store_memory`** + **`must_apply`** surface durable facts on similar future tasks |
| Skill library sprawl | **`agent-brain add owner/repo`** installs and indexes packages (ECC, team rules) |
| Two laptops / backup | **Git + encrypted cloud sync** for `brain.db` bundles |
| MCP disconnects block you | **Scoped gate (v0.7.1+)** — Shell/Read keep working; offline cooldown instead of hard-lock |
| CLI asks MCP approval every run | **`permissions.json`** (v0.7.2+) — one-time `agent-brain:*` allowlist |

You still use Cursor Agent mode. agent-brain does not replace the model — it **chooses context and enforces the gate** before the agent acts.

---

## How this is different from similar tools

| Product / approach | What it optimizes | What it does *not* do | agent-brain |
|--------------------|-------------------|------------------------|-------------|
| **Cursor skills / rules (static)** | Authoring & discovery | Per-turn budget, hard enforcement, ranked retrieval | Routes + hooks + token cap every message |
| **Memory SaaS** (Mem0, Zep, etc.) | Long-term chat / user memory | Local skill libraries, IDE hooks, package installs | Local SQLite memory *plus* skill/agent/rule routing |
| **Agent frameworks** (LangGraph, CrewAI) | Multi-step agent runtime in *your app* | Drop-in MCP gate for the IDE | Zero app code — MCP + hooks in Cursor |
| **Vector / RAG** (Chroma, Qdrant, etc.) | Document search at scale | Phase-aware skills, `must_apply`, negative memory, package scope | BM25 + embeddings tuned for *skills*, not generic docs |
| **“Just add a rule”** | Free | Cannot stop the model from ignoring it | Hooks deny tools until `route_task` runs |

### Scenario: which tool wins?

- **“I have 200 ECC skills and the agent keeps using the wrong ones”** → agent-brain ranks by message + phase and returns paths to load.
- **“The agent skips `route_task` and greps the repo”** → hooks deny Shell/Read until routing succeeds (scoped to agent-brain MCP tools by default).
- **“We agreed to use Vitest, but next week it suggested Jest again”** → `store_memory` + negative/`must_apply` memory resurfaces on test-related prompts.
- **“Mem0 remembers chat; I need *project conventions* in the IDE”** → structured facts in local `brain.db`, synced optionally via git/cloud — no API key per turn.
- **“I want skills on laptop + desktop”** → `sync git` or encrypted `sync cloud` bundles, not copy-pasting `.cursor/rules`.

**When to use it:** you have (or want) a large skill library — ECC, superpowers, team rules — and need **fast, repeatable, enforced** context every turn.

**When not to use it:** a handful of project rules already work; you only need chat recall (not skill routing); you do not use Agent mode or MCP.

---

## What you get (main features)

| Feature | Copy-paste setup | Why use it |
|---------|------------------|------------|
| **Turn routing** | MCP `route_task` (automatic via hooks) | Right skills under token budget every message |
| **Hook enforcement** | `agent-brain install --global` | Hard gate — not “please follow the rule” |
| **Skill packages** | `agent-brain add affaan-m/ecc` | One command to install + index hundreds of skills |
| **Durable memory** | Agent calls `store_memory` at task end | Conventions persist across sessions |
| **Git sync** | `config.yaml` + `sync git init` | Second machine pulls same brain bundle |
| **Cloud sync** | `config.yaml` + `AGENT_BRAIN_SYNC_KEY` | Encrypted off-site backup (S3/R2/MinIO) |
| **Auto-update** | `~/.agent_brain/config.yaml` | Package + MCP binary updates while `serve` runs |
| **CLI MCP allowlist** | `~/.cursor/permissions.json` | Cursor CLI stops prompting every agent session |
| **Resilience** | Default hook scope `brain_mcp` | MCP down ≠ entire session frozen |
| **Observability** | `agent-brain briefing`, `last-route.md` | See what was routed without parsing MCP JSON |

**Full operator guide:** [docs/USAGE.md](docs/USAGE.md)

---

## Complete setup (copy & paste)

### 1. Install the binary

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/master/scripts/install.sh | bash -s -- --global
```

Or from source:

```bash
cargo install --git https://github.com/aeswibon/agent-brain --locked agent-brain
agent-brain install --global
```

`install --global` writes MCP config, hooks, permissions, and the project rule. **You do not run `serve` manually** — Cursor spawns it.

### 2. Cursor MCP — `~/.cursor/mcp.json`

Replace `/Users/you/.local/bin/agent-brain` with your path (`which agent-brain`). If the file already exists, merge the `agent-brain` entry under `mcpServers`:

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
        "AGENT_BRAIN_SESSION_INGEST_DELAY_SEC": "180",
        "AGENT_BRAIN_ROUTE_GATE_SCOPE": "brain_mcp",
        "AGENT_BRAIN_ROUTE_OFFLINE_SECS": "1800"
      }
    }
  }
}
```

| Env var | Default | Purpose |
|---------|---------|---------|
| `AGENT_BRAIN_ROUTE_GATE_SCOPE` | `brain_mcp` | Gate only agent-brain MCP tools (`all` = legacy strict mode) |
| `AGENT_BRAIN_ROUTE_OFFLINE_SECS` | `1800` | After MCP disconnect, stop blocking other tools for 30m |

Or re-run (merges safely):

```bash
agent-brain install --global
```

### 3. CLI MCP auto-approve — `~/.cursor/permissions.json`

Required for **Cursor CLI** agents (separate from hooks). Merge into existing file or create new:

```json
{
  "mcpAllowlist": ["agent-brain:*"]
}
```

Then in Cursor: **Settings → enable Run Mode** (allowlist only applies with Run Mode on).

`install --global` appends `agent-brain:*` without removing entries like `github:*`.

One-off CLI scripts: `cursor agent --approve-mcps` or `--force`.

### 4. Hooks + rule (automatic)

`install --global` installs:

- `~/.cursor/hooks/agent-brain/route_gate.py`
- Merged entries in `~/.cursor/hooks.json` (`beforeSubmitPrompt`, `preToolUse`, `beforeMCPExecution`, …)
- `.cursor/rules/agent-brain.mdc` in each project (requires `route_task` every turn)

Example hook entries (for reference — installer merges these):

```json
{
  "version": 1,
  "hooks": {
    "beforeSubmitPrompt": [
      { "command": "./hooks/agent-brain/route_gate.py", "matcher": "UserPromptSubmit" }
    ],
    "preToolUse": [
      { "command": "./hooks/agent-brain/route_gate.py" }
    ],
    "beforeMCPExecution": [
      { "command": "./hooks/agent-brain/route_gate.py", "failClosed": true }
    ],
    "afterMCPExecution": [
      { "command": "./hooks/agent-brain/route_gate.py" }
    ]
  }
}
```

Requires `python3` on PATH. Disable hooks for debugging only: `"AGENT_BRAIN_ROUTE_HOOKS": "0"` in MCP env.

### 5. Enable in Cursor (checklist)

1. **Restart Cursor** (or Reload Window) after install
2. **Settings → MCP** → enable **agent-brain** (green status)
3. **Settings → Hooks** → confirm agent-brain hooks listed
4. **Settings → Rules** → confirm `agent-brain.mdc` (always apply)
5. **Settings → Run Mode** → on (for CLI MCP allowlist)
6. Open chat in **Agent mode** (not Ask-only)

### 6. Verify

```bash
agent-brain version
agent-brain doctor
agent-brain briefing    # after first route_task in Cursor
```

Expected: MCP path matches binary, hooks present, permissions include `agent-brain:*`. If `doctor` reports stale serve after a local rebuild:

```bash
agent-brain install --global --reload
```

---

## Enable main features (copy & paste)

### Skill packages (ECC)

```bash
agent-brain add affaan-m/ecc
agent-brain package list
agent-brain package update ecc
```

Clones to `~/.agent_brain/packages/ecc/` and indexes skills, agents, rules, commands. Re-index after updates: `agent-brain index` (optional — bootstrap also indexes on MCP start).

### Durable memory

The **agent** calls this at task end (you do not run it manually in normal use). Example MCP payload:

```json
{
  "topic": "testing",
  "fact": "Use Vitest, not Jest, for this monorepo.",
  "scope": "project",
  "scope_key": "/path/to/repo",
  "confidence": 0.9
}
```

Rules: max 50 words, no secrets. Conflicting facts are superseded with a conflict log (`agent-brain sync status`).

### v0.10 operator loop (promote, gc, digest, eval)

```bash
# 1) Stage skill drafts from memory facts (human approval required)
agent-brain promote list
agent-brain promote approve <staging-id>
agent-brain promote reject <staging-id>

# 2) Memory garbage collection (dry-run by default)
agent-brain memory gc
agent-brain memory gc --apply

# 3) Weekly operator digest from retrieval logs
agent-brain digest --weekly

# 4) CI retrieval quality gate
agent-brain eval --ci
```

Promotion writes draft `SKILL.md` files to `~/.agent_brain/staging/` first; approval is explicit before files land in `.cursor/skills/`.

### Auto-update — `~/.agent_brain/config.yaml`

```bash
agent-brain config init   # optional helper
```

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
    recheck_interval_minutes: 15
    restart_idle_secs: 10
    restart_max_wait_secs: 300
    restart_min_delay_secs: 2

sync:
  git:
    remote: git@github.com:you/agent-brain-sync.git
    branch: main
    auto_push: true
  cloud:
    enabled: false
    provider: s3
    bucket: my-agent-brain-backup
    key: brain-sync.tar.zst.age
    encrypt: true
    encryption_key_env: AGENT_BRAIN_SYNC_KEY
    region: auto
    auto_push: false

memory_gc:
  stale_days: 90
  very_stale_days: 180
```

MCP `serve` checks updates in the background; after a binary update it restarts when idle so Cursor reconnects.

### Git sync (two machines)

**Machine A** — init and push:

```bash
export AGENT_BRAIN_HOME=~/.agent_brain   # default

agent-brain sync git init --remote git@github.com:you/agent-brain-sync.git
agent-brain sync git push
```

**Machine B** — clone and pull (branch from `config.yaml` → `sync.git.branch`):

```bash
agent-brain sync git clone --remote git@github.com:you/agent-brain-sync.git
agent-brain sync git pull
```

Status and conflicts:

```bash
agent-brain sync status
agent-brain sync restore <conflict-id>
```

With `sync.git.auto_push: true` in config, successful `store_memory` can push automatically.

### Encrypted cloud sync (S3 / R2 / MinIO)

```bash
# 32+ char passphrase — store in OS keychain or env
export AGENT_BRAIN_SYNC_KEY='your-long-random-passphrase-here!!'

agent-brain secrets setup AGENT_BRAIN_SYNC_KEY
```

`~/.agent_brain/config.yaml` (cloud section):

```yaml
sync:
  cloud:
    enabled: true
    provider: s3
    bucket: my-bucket
    key: brain-sync.tar.zst.age
    encrypt: true
    encryption_key_env: AGENT_BRAIN_SYNC_KEY
    region: auto
    endpoint: ""          # set for R2/MinIO, e.g. https://<account>.r2.cloudflarestorage.com
    auto_push: true
```

Push / pull:

```bash
agent-brain sync cloud push
agent-brain sync cloud pull
```

Bundle is tar.zst + age encryption. Secret **names** sync in bundles; values stay in the OS keychain (`agent-brain secrets status`).

### Observability

After any routed turn:

```bash
cat ~/.agent_brain/logs/last-route.md
agent-brain briefing
```

### Session digests (Cursor, Codex, Gemini, OpenCode)

Background ingest runs during MCP bootstrap. Manual ingest:

```bash
agent-brain sessions status
agent-brain sessions ingest
agent-brain sessions ingest --source gemini,opencode
```

Sources scanned:

| Source | Path |
|--------|------|
| Cursor | `~/.cursor/projects/**/agent-transcripts/**/*.jsonl` |
| Codex | `~/.codex/sessions/**/*.jsonl` |
| Gemini | `~/.gemini/**/transcript.jsonl` |
| OpenCode | `~/.local/share/opencode/opencode.db` |

Each session becomes one low-priority memory fact: `session-digest-{source}-{slug}`.

In Cursor: **View → Output → MCP** (select `agent-brain`) for one-line stderr summaries (`latency_ms`, `cache_hit`, `briefing`).

### After a local rebuild

```bash
cargo install --path agent-brain --force
agent-brain install --global --reload
# Reload Cursor window or toggle MCP once
agent-brain doctor
```

---

## How each turn works

```text
user message
  → hook marks turn “needs route_task”
  → agent calls route_task (blocked until this succeeds)
  → BM25 prefilter → embed → score → token budget assembly
  → agent loads recommended skills/agents from returned paths
  → agent applies rules + must_apply memory
  → agent does work (Shell, Read, Edit, …)
  → agent calls store_memory for durable decisions
```

| Action | Who |
|--------|-----|
| Start MCP | Cursor spawns `agent-brain serve` |
| Index skills/rules | agent-brain on startup (background) |
| Route each turn | Agent → `route_task` (hook-gated) |
| Persist decisions | Agent → `store_memory` at task end |

**Write safety (v0.7.2+):** all DB mutations serialize through one write queue — imports and sync pulls cannot corrupt active stores. [agent-brain/docs/concurrency.md](agent-brain/docs/concurrency.md)

---

## Lower latency

`route_task` target: **under 50ms** on warm cache.

| Mechanism | Effect |
|-----------|--------|
| Turn cache (60s LRU) | Repeat queries ~0ms |
| BM25 fast path | Skip embed when FTS hits are strong |
| Bootstrap prewarm | Warm index + embedder before first route |
| Query embedding cache | Persisted in SQLite across restarts |

Tune via MCP `env` in `mcp.json` — see [docs/USAGE.md](docs/USAGE.md#environment-variables).

---

## Other MCP hosts

Same binary works with **Cursor, Claude Code, OpenCode, Claude Desktop, and VS Code**. Use host installers:

```bash
agent-brain install --claude-desktop
agent-brain install --vscode [--global]
agent-brain install --claude-code [--global]
agent-brain install --opencode [--global]
agent-brain install --all
```

Skills under `~/.claude/` and `~/.codex/` are already indexed.

```json
{
  "mcpServers": {
    "agent-brain": {
      "command": "/absolute/path/to/agent-brain",
      "args": ["serve"]
    }
  }
}
```

Add a host rule: call **`route_task`** every turn before other tools.

---

## Commands

| Command | Description |
|---------|-------------|
| `agent-brain install --global` | MCP + hooks + permissions + project rule |
| `agent-brain install --global --reload` | Same + bump MCP build stamp after rebuild |
| `agent-brain add <owner/repo>` | Install skill package |
| `agent-brain package list\|update\|remove` | Manage packages |
| `agent-brain sync git\|cloud …` | Multi-machine brain sync |
| `agent-brain secrets setup\|status` | Keychain-backed secret refs |
| `agent-brain doctor` | MCP path, hooks, stale serve, sync health |
| `agent-brain briefing` | Last route summary |
| `agent-brain index` | Force reindex |
| `agent-brain version` | Installed version |
| `agent-brain serve` | Manual MCP (debug only) |

---

## Data directory

`~/.agent_brain/` (override with `AGENT_BRAIN_HOME`). First MCP start downloads the embedding model (~90MB).

---

## Development

```bash
cargo test --release -p agent-brain
make release-macos             # macOS: build + adhoc sign (required for Cursor MCP)
cargo build --release -p agent-brain
python3 agent-brain/hooks/test_route_gate.py
```

On macOS, release CI artifacts and `install.sh` adhoc-sign binaries; local builds need `make release-macos` or `agent-brain doctor --fix`.

---

## Releases

See [CHANGELOG.md](CHANGELOG.md). Tags `v*` publish platform binaries with changelog-based release notes.

## License

MIT
