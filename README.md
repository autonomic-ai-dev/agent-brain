# agent-brain

**Local context & memory engine — route ~500 tokens of the right skills from thousands, with hooks that make it mandatory.**

agent-brain is a token-efficient context engine: skills/rules routing, temporal memory, multi-signal retrieval, and hook-enforced `route_task` — all in one binary, zero external vector DB.

Rust is the brain; agents are the hands.

```bash
curl -fsSL https://raw.githubusercontent.com/autonomic-ai-dev/agent-brain/master/scripts/install.sh | bash -s -- --global --with-starter
agent-brain onboarding
```

**MCP is live immediately** — `serve` starts stdio first; index sync, session ingest, and prewarm run in a background thread by default.

**Proof narrative:** [before/after blog](docs/blog/before-and-after-agent-brain.md) · [benchmarks](docs/benchmarks/) · [team workflow](docs/TEAM-WORKFLOW.md)

---

## Why agent-brain?

Three problems every power-user hits with large skill libraries:

1. **Context bloat** — hundreds of skills and rules cannot all fit in one turn. Stuffing them in degrades reasoning and burns tokens.
2. **Soft enforcement** — telling the model "use skills first" in a rule is optional. The agent can still grep, edit, or guess.
3. **No durable routing memory** — decisions from last week are not automatically surfaced as constraints on the next similar task.

**agent-brain fixes this with a local context engine:**

| Problem | agent-brain answer |
|---------|-------------------|
| Too much to load | **`route_task`** returns ~500 tokens of the *right* skills/rules/memory for *this* message |
| Model skips skills | **Hooks** block agent-brain MCP tools until `route_task` succeeds each turn |
| Forgotten conventions | **ADD-only `store_memory`** + **`must_apply`** + temporal validity windows |
| Memory loses history | **Temporal KG** in SQLite (`memory_kg_edges`, `valid_from` / `invalid_at`) — facts evolve, not erase |
| Slow vector search under scope filters | **Filterable HNSW** (embedded, Qdrant-inspired) — scope-aware ANN with bridge edges |
| Skill library sprawl | **`agent-brain add owner/repo`** installs and indexes packages (ECC, team rules) |
| Two laptops / backup | **Git + encrypted cloud sync** for `brain.db` bundles |
| MCP disconnects block you | **Scoped gate (v0.7.1+)** — other tools keep working; offline cooldown instead of hard-lock |
| CLI asks MCP approval every run | **`permissions.json`** (v0.7.2+) — one-time `agent-brain:*` allowlist |

agent-brain does not replace the model — it **chooses context and enforces the gate** before the agent acts.

---

## Quick Install

```bash
curl -fsSL https://raw.githubusercontent.com/autonomic-ai-dev/agent-brain/master/scripts/install.sh | bash -s -- --global
```

Or from source:

```bash
cargo install --git https://github.com/autonomic-ai-dev/agent-brain --locked agent-brain
agent-brain install --global
```

`install --global` writes MCP config, hooks, permissions, and the project rule. You do not run `serve` manually — your MCP host spawns it.

Verify:

```bash
agent-brain version
agent-brain doctor
```

Full setup and configuration guide: [docs/USAGE.md](docs/USAGE.md)

---

## Main features

| Feature | Setup | Why use it |
|---------|-------|------------|
| **Turn routing** | MCP `route_task` (automatic via hooks) | Right skills under token budget every message |
| **Hook enforcement** | `agent-brain install --global` | Hard gate — not "please follow the rule" |
| **Skill packages** | `agent-brain add @nextjs` or `owner/repo` | Curated aliases + one-command installs |
| **Durable memory** | Agent calls `store_memory` at task end | Conventions persist across sessions |
| **Git sync** | `config.yaml` + `sync git init` | Second machine pulls same brain bundle |
| **Cloud sync** | `config.yaml` + `AGENT_BRAIN_SYNC_KEY` | Encrypted off-site backup (S3/R2/MinIO) |
| **Auto-update** | `~/.agent_brain/config.yaml` | Package + MCP binary updates while `serve` runs |
| **CLI MCP allowlist** | `permissions.json` | Stops prompting every agent session |
| **Resilience** | Default hook scope `brain_mcp` | MCP down does not freeze your session |
| **Observability** | `agent-brain briefing`, `last-route.md` | See what was routed + est. token savings |

**Memory engine (v0.21+):** ADD-only facts with temporal validity windows; multi-signal retrieval fusion (semantic + BM25 + lexical + entity overlap); filterable HNSW in-memory ANN; session digests across MCP hosts; observation engine that auto-synthesizes must_apply candidates from recurring facts; BEAM eval for recall, temporal, and must_apply suites; trace extraction from shell and package-manager logs.

**Operator loop:** `agent-brain promote list/approve/reject` stages skill drafts from memory facts. `agent-brain memory gc --apply` deduplicates, prunes stale facts, and vacuum-packs the KG. `agent-brain digest --weekly` generates operator summaries from retrieval logs. `agent-brain eval --ci` runs the retrieval quality gate.

Skill packages: `agent-brain registry list` · `agent-brain add @supervisor` · `@starter` — see [docs/registry/README.md](docs/registry/README.md).

Full operator guide: [docs/USAGE.md](docs/USAGE.md) · Architecture deep dive: [docs/architecture/README.md](docs/architecture/README.md)

---

## Commands

| Command | Description |
|---------|-------------|
| `agent-brain install --global` | MCP + hooks + permissions + project rule |
| `agent-brain install --global --reload` | Same + bump MCP build stamp after rebuild |
| `agent-brain add <owner/repo>` | Install skill package |
| `agent-brain package list\|update\|remove` | Manage packages |
| `agent-brain sync git\|cloud ...` | Multi-machine brain sync |
| `agent-brain secrets setup\|status` | Keychain-backed secret refs |
| `agent-brain doctor` | MCP path, hooks, stale serve, sync health |
| `agent-brain briefing` | Last route summary |
| `agent-brain index` | Force reindex |
| `agent-brain version` | Installed version |
| `agent-brain serve` | Manual MCP (debug only) |

---

## Benchmarks

Reproducible proof artifacts in [`docs/benchmarks/`](docs/benchmarks/). Regenerate locally with `cargo run --release -p agent-brain -- proofs --ci`.

| Gate | Index size | Golden cases | Recall@3 | Threshold | CI |
|------|------------|--------------|----------|-----------|-----|
| Isolated skills + memory | 500 skills | 10 | **1.00** (10/10) | >= 0.85 | `stage-test.yml` |
| skills.sh catalog | **2000 real** skills.sh skills | **50** | **1.00** (50/50) | >= 0.80 | `stage-skills-sh-eval.yml` |
| Warm-route p95 (fixture) | 500 | — | **<= 100 ms** | gated | `proofs --ci` |
| ANN scale warm-route p95 | 1k / 5k / 10k | — | **<= 50 ms** each | gated (1k CI) | `bench --scale --full` |
| Graphify ingest | 1k nodes | — | **<= 2 s** | gated (CI) | `bench --graphify` |
| Graphify `code_context` route p95 | 1k nodes + 500 skills | — | **<= 65 ms** | gated (CI) | `bench --graphify` |
| MCP tool latency | 500 skills | route + context + token tools | see `bench --mcp` | informational | `bench --mcp` |
| Turn-cache p95 (fixture) | 500 | — | **<= 30 ms** | gated | `proofs --ci` |
| Filterable HNSW p95 | 2k scoped | — | **<= 50 ms** | gated (unit test) | `ann` tests |
| Execution supervisor | 500 + `@supervisor` | 3 scenarios | skill **100%** · must_apply **100%** · savings **~99%** · p95 **<= 100 ms** | gated | `proofs --ci` |
| Token MCP tools | synthetic 2k-line file | 4 tools | **>= 80%** savings vs full read | gated | `proofs --ci` |
| Supervisor telemetry | hook + tool_log | 24h window | tool savings + Read steers in briefing | informational | `agent-brain stats` |
| Anti-pattern loop | hook steer | — | `suggest-memory approve` -> negative memory | manual | CLI |
| Hook gate logic | — | — | **< 1 ms p95** | gated | `test_route_gate.py` |

**skills.sh eval** runs against committed `fixture-2k.db` — no network, no synthetic fillers. See [docs/benchmarks/skills-sh/README.md](docs/benchmarks/skills-sh/README.md).

**Token savings (every turn):** `agent-brain briefing` and `~/.agent_brain/logs/last-route.md` show routed tokens vs an estimated naive full-index load (~120 tok/item). On a 2000-skill index routing ~500 tokens, that is typically **~99% fewer tokens** than loading everything.

**Before/after (2000 skills):** [docs/blog/before-and-after-agent-brain.md](docs/blog/before-and-after-agent-brain.md) · [Team workflow](docs/TEAM-WORKFLOW.md)

---

## Other MCP hosts

Same binary works with Cursor, Claude Code, Codex, OpenCode, Claude Desktop, and VS Code:

```bash
agent-brain install --claude-desktop
agent-brain install --vscode [--global]
agent-brain install --claude-code [--global]
agent-brain install --opencode [--global]
agent-brain install --codex [--global]
agent-brain install --gemini [--global]
agent-brain install --antigravity [--global]
agent-brain install --all --global
```

Skills under `~/.claude/` and `~/.codex/` are already indexed.

---

## Data directory

`~/.agent_brain/` (override with `AGENT_BRAIN_HOME`). First MCP start downloads the embedding model (~90MB).

---

## Development

```bash
cargo test --release -p agent-brain
make release-macos             # macOS: build + adhoc sign
cargo build --release -p agent-brain
python3 agent-brain/hooks/test_route_gate.py
```

On macOS, release CI artifacts and `install.sh` adhoc-sign binaries; local builds need `make release-macos` or `agent-brain doctor --fix`. If Cursor blocks MCP after a browser download, run `xattr -cr ~/.local/bin/agent-brain && codesign --force --sign - ~/.local/bin/agent-brain`.

---

## Releases

See [CHANGELOG.md](CHANGELOG.md). Tags `v*` publish platform binaries with changelog-based release notes.

## License

MIT
