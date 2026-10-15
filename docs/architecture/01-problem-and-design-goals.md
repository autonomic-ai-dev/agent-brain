# 1. Problem and design goals

## Summary

agent-brain is a **local turn router** for IDE agents. It answers one question per user message: *given hundreds of skills, rules, agents, and memories on disk, what is the smallest high-signal context to load under a token budget?*

It does **not** replace the model, run workflows, or host chat history in the cloud.

## The three problems we set out to solve

### 1. Context bloat

Power users accumulate large skill libraries (ECC, superpowers, team rules, plugin agents). Loading “everything relevant” into the system prompt or @-mentioning skills manually does not scale:

- Token cost grows linearly with library size
- The model attends poorly to long, undifferentiated skill lists
- Different tasks need different subsets (review vs implement vs debug)

**Design response:** `route_task` returns ranked **pointers** (paths + short rationale), not full file bodies, capped by `max_tokens` and per-type `RouteLimits`.

### 2. Soft enforcement

Cursor rules that say “call route_task first” are suggestions. The agent can still grep, edit, or improvise without loading the right skill.

**Design response:** Host **hooks/plugins** (`route_gate.py`, OpenCode plugin, Claude/Gemini settings) deny other agent-brain MCP tools until `route_task` succeeds for the current user turn. Rules and MCP tool descriptions are backup layers, not the primary gate.

### 3. No durable routing memory

Chat transcripts forget project conventions. “We use Vitest, not Jest” must be rediscovered or re-stated.

**Design response:** Structured **facts** in local SQLite (`store_memory`), scoped by project/global, with deduplication, supersession, negative polarity, and optional `apply_when` conditions.

## North star (from the master spec)

> One sub-50ms local call per turn that surfaces the right agents, skills, rules, and memory under a strict token budget.

Latency targets are aspirational on cold embed; warm paths (turn cache, query-embedding cache, in-memory search snapshot) aim for single-digit milliseconds on repeat queries.

## Design goals matrix

| Goal | Mechanism | Non-goal |
|------|-----------|----------|
| **Fast** | Turn cache, BM25 prefilter, parallel BM25 + embed, background bootstrap | Sub-ms at any cost on cold ONNX load |
| **Local-first** | SQLite + on-device `fastembed` | Cloud vector DB for core routing |
| **Complete index** | Walk all configured skill/agent/rule roots | Duplicate skill authoring UI |
| **Zero-touch routing** | Hooks + MCP tool schema | User manually picking skills each turn |
| **Durable conventions** | `store_memory`, conflict log | Full conversation RAG |
| **Portable brain** | Git/cloud sync bundles | Multi-user SaaS memory |

## How this differs from adjacent tools

| Approach | Optimizes for | Why not enough alone |
|----------|---------------|----------------------|
| Static Cursor rules | Authoring | No per-turn budget or ranking |
| Memory SaaS (Mem0, Zep) | Chat/user recall | No IDE hook gate; no local skill index |
| Agent frameworks (LangGraph) | App-runtime orchestration | Not drop-in for Cursor Agent mode |
| Vector DB (Chroma, Qdrant) | Document search at scale | No phase, `must_apply`, negative facts, package scope |
| “Just add a better rule” | Free | Cannot block tools |

agent-brain composes **retrieval + enforcement + local structured memory** for the IDE agent harness, not for a custom backend app.

## Alternatives considered at product level

### A. Cloud-hosted router API

**Idea:** Central service indexes skills and returns routes.

**Rejected because:** Adds latency, privacy concerns for repo paths and memory, offline failure, and per-turn API cost. Local SQLite + ONNX embeddings keep the critical path on-device.

### B. Pure rules / no MCP

**Idea:** Encode all routing in `.cursor/rules` with @file references.

**Rejected because:** Rules cannot enforce tool order; they grow unbounded; no feedback loop or memory dedup.

### C. Fork Cursor / patch the IDE

**Rejected because:** Not maintainable across Cursor versions; MCP + hooks are the supported extension surface.

### D. Embed entire skill bodies in route response

**Rejected because:** Defeats the token budget; agents should `Read` skill files after routing picks paths.

## Trade-offs

- **Complexity:** Users must install binary, MCP, and (for Cursor) hooks — more moving parts than a single rule file.
- **Host asymmetry:** Cursor/Claude Code/OpenCode/Codex/Gemini-Antigravity install deny gates; VS Code and Claude Desktop remain instruction-only (see [07](07-enforcement-and-multi-host.md)).
- **Index freshness:** Skills on disk are indexed on bootstrap interval, not instantly on every file save (configurable).

## For senior engineers and principal architects

### System boundary (what we are not building)

agent-brain sits in a narrow layer of the agent stack:

```text
User intent
    → IDE agent harness (Cursor, OpenCode, …)
        → agent-brain: rank + enforce + remember (this project)
            → LLM + tools (Read, Shell, upstream MCPs)
```

We **do not** own: chat history, workflow DAGs, code execution, model selection, or team RBAC. That boundary is intentional — every feature request should be tested against “is this still a turn router?” If not, it belongs in the harness, a framework, or a separate service.

### Invariants we optimize for

| Invariant | Why it matters |
|-----------|----------------|
| **Local critical path** | Routing must work offline, without API keys, on a laptop |
| **Bounded output** | `max_tokens` + per-type limits are hard contracts, not hints |
| **Enforceable first step (Cursor)** | Soft instructions failed in production; hooks are the product wedge |
| **Durable conventions ≠ chat** | Memory is atomic facts, not transcript RAG |
| **Verifiable routing** | USP is accuracy; without eval gates, quality rots silently |

### Adoption thesis

Power users already have **hundreds of skills** and **dozens of rules**. The failure mode is not “missing one skill” — it is **wrong context at scale**: wrong skill loaded, right skill never read, repeated debates about stack choices. agent-brain bets that:

1. **Ranking under budget** beats “load everything in rules”
2. **Hard gates on major CLIs** beat “please call route_task”
3. **Local hybrid retrieval** beats cloud memory SaaS for IDE latency and privacy

If any of those bets is false for your org (e.g. you standardize on one tiny rule set, or you forbid hooks), adoption value drops sharply.

### Risk register (design-level)

| Risk | Mitigation today | Residual |
|------|------------------|----------|
| Wrong skill routed | Hybrid retrieval + skill golden eval | Paraphrase edge cases, opaque skill names |
| Agent bypasses router | Deny gates on Cursor/Claude/OpenCode/Codex/Gemini-Antigravity; doctor `--fix` | VS Code / Claude Desktop still advisory |
| Memory pollution | Dedup, GC, negative protection | Session ingest noise |
| Stale index | `index_version` + bootstrap interval | Edits within TTL may lag |
| MCP offline freezes session | Scoped gate + offline grace | User may proceed without route |

### Questions a PE should ask before adopting

1. **Library size:** Do developers have enough skills/rules that manual @-mention does not scale?
2. **Host mix:** Are agents on gated CLIs, or mostly advisory hosts (VS Code / Claude Desktop)?
3. **Privacy:** Are project paths and conventions allowed to stay on-device?
4. **Ops appetite:** Will someone run `memory gc`, `eval --ci`, and sync periodically?
5. **Accuracy bar:** Is Recall@3 ≥ 0.85 on your golden tasks acceptable, or do you need per-team eval sets?

### Evolution if constraints change

| If… | Likely design shift |
|-----|---------------------|
| Index → 50k+ items | ANN index (sqlite-vec / sidecar), cross-encoder rerank |
| Team-wide brain | Server-side router + auth (violates current local-first) |
| Hooks everywhere | Per-host adapter layer; same gate semantics |
| LLM cost → zero | Optional LLM reranker stage (today rejected on latency) |

## Further reading

- [02-system-overview.md](02-system-overview.md) — component map
- [README.md](../../README.md) — scenario comparison table
- [Master spec](../superpowers/specs/mcp_router_master_spec.md) — original Phase 1/2 split
