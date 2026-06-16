# 11. Decisions log

Quick reference for major design choices. Each row links to the article with full context.

| Decision | Chosen | Alternatives rejected | Article |
|----------|--------|----------------------|---------|
| Core transport | MCP stdio | HTTP-only, LSP, IDE extension | [02](02-system-overview.md) |
| Storage | SQLite + FTS5 + BLOB embeddings | Qdrant, vectors.bin, JSON facts | [03](03-local-first-storage.md) |
| Route output | Paths + rationale | Full skill bodies in JSON | [04](04-turn-routing-and-retrieval.md) |
| Retrieval | Hybrid BM25 + embed + lexical | BM25-only fast path, LLM router | [04](04-turn-routing-and-retrieval.md) |
| Memory shape | Scoped atomic facts | Full transcripts | [05](05-memory-model.md) |
| Skill discovery | Filesystem walk | Central registry API | [06](06-indexing-and-packages.md) |
| Enforcement | Cursor hooks + scoped gate | Rules-only, gate-all-tools | [07](07-enforcement-and-multi-host.md) |
| Multi-host | MCP + instruction files | Same hooks everywhere | [07](07-enforcement-and-multi-host.md) |
| Portability | Git/cloud bundles | Live DB sync | [08](08-sync-sessions-portability.md) |
| Upstream MCP | Explicit `route_to_mcp` | Auto-proxy all tools | [09](09-upstream-and-operator-loop.md) |
| Skill from memory | Staged promote + approve | Auto-write SKILL.md | [09](09-upstream-and-operator-loop.md) |
| Writes | Single write queue | Parallel DB writers | [10](10-concurrency-and-performance.md) |
| Embed cache | `~/.agent_brain/cache/fastembed` | Project cwd `.fastembed_cache` | [03](03-local-first-storage.md) |
| Language/runtime | Rust | TypeScript MCP server | [02](02-system-overview.md) |

## Deferred (not rejected forever)

| Idea | Status | Notes |
|------|--------|-------|
| Cross-encoder reranker | Deferred | Accuracy vs latency |
| Filesystem watch for index | Deferred | Bootstrap interval sufficient for now |
| Skill-level `apply_when` triggers | Partial | Memory has it; skills use description + search |
| OpenCode / Claude hooks | Blocked on host | No deny-hook API yet |
| Skill golden eval set | **Shipped** | `proofs --ci` on isolated fixture |
| Published proof artifact | **Shipped** | `docs/benchmarks/latest.json` |
| `vectors.bin` sidecar | Spec only | Embeddings in SQLite today |

## How to propose a change

1. State the **user-visible problem** (not only implementation preference).
2. List at least **two alternatives** with trade-offs.
3. If changing retrieval or memory semantics, extend **`eval --ci`** or integration tests.
4. Update the relevant architecture article **and** add a PE subsection if the change affects invariants, failure modes, or adoption thesis.
5. Add a row to this decisions log.

## For principal architects (using this log)

This table is the **fast audit trail** for design reviews. When evaluating a fork or internal wrapper:

- **Red flags:** Changes that violate local-first, bounded output, or eval gates without replacing them
- **Green flags:** Changes that extend golden suites, index quality, or host install coverage
- **Deferred items** are not weaknesses — they are explicit **latency vs accuracy** or **host API** bets

Pair this log with [12-routing-accuracy.md](12-routing-accuracy.md) for the adoption-critical USP and [01-problem-and-design-goals.md](01-problem-and-design-goals.md) for boundary definition.

## Version note

This log reflects the codebase through **v0.11** (hybrid retrieval, GC reporting, multi-host install, operator loop). See [CHANGELOG.md](../../CHANGELOG.md) for release-level history.

## Further reading

- [README.md](README.md) — series index
- [Master spec](../superpowers/specs/mcp_router_master_spec.md) — original roadmap (some items since shipped)
