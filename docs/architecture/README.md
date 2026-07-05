# Architecture documentation series

This series explains **why agent-brain exists**, **how it is built**, and **what alternatives were considered** for each major decision.

**Primary audience:** senior engineers and principal engineers evaluating adoption, extending the router, or reviewing design trade-offs. Contributors and power users can read the same material; operators who only need commands should start with [USAGE.md](../USAGE.md).

The README tells you *how to run* agent-brain. These documents tell you *why it works the way it does* — and what we would change if constraints were different.

## How to read this series

Each article follows a consistent shape:

1. **What we built** — concrete behavior you can verify in the repo
2. **Why this way** — constraints and goals that drove the design
3. **Alternatives considered** — options we rejected or deferred (with reasons)
4. **Trade-offs** — what you give up, and known limitations
5. **For senior engineers / PEs** — invariants, failure modes, evaluation questions, scale evolution

### Reading paths

| If you are… | Start here | Then |
|-------------|------------|------|
| **PE / architect** deciding adopt vs build | [01](01-problem-and-design-goals.md), [12](12-routing-accuracy.md), [11](11-decisions-log.md) | [04](04-turn-routing-and-retrieval.md), [07](07-enforcement-and-multi-host.md) |
| **Senior dev** integrating or debugging | [02](02-system-overview.md), [04](04-turn-routing-and-retrieval.md) | Module cited in the article you're changing |
| **Operator** running sync/GC/eval | [09](09-upstream-and-operator-loop.md), [08](08-sync-sessions-portability.md) | [USAGE.md](../USAGE.md) |
| **Contributor** changing retrieval or memory | [12](12-routing-accuracy.md), [04](04-turn-routing-and-retrieval.md), [05](05-memory-model.md) | Extend `eval --ci` before merging |

Read in order for a full picture, or jump to a topic:

| # | Article | Topics |
|---|---------|--------|
| 1 | [Problem and design goals](01-problem-and-design-goals.md) | Context bloat, soft rules, durable memory; north star |
| 2 | [System overview](02-system-overview.md) | Components, request flow, Rust + MCP + hooks |
| 3 | [Local-first storage](03-local-first-storage.md) | SQLite, embeddings, scopes, schema evolution |
| 4 | [Turn routing and retrieval](04-turn-routing-and-retrieval.md) | `route_task`, hybrid search, phase, cache |
| 5 | [Memory model](05-memory-model.md) | Facts, dedup, negative memory, `apply_when`, feedback |
| 6 | [Indexing and packages](06-indexing-and-packages.md) | Bootstrap, index roots, ECC packages |
| 7 | [Enforcement and multi-host](07-enforcement-and-multi-host.md) | Hooks, gate scope, OpenCode/Claude install |
| 8 | [Sync, sessions, portability](08-sync-sessions-portability.md) | Git/cloud sync, session digest, second machine |
| 9 | [Upstream and operator loop](09-upstream-and-operator-loop.md) | Federation, promote, GC, digest, eval |
| 10 | [Concurrency and performance](10-concurrency-and-performance.md) | Write queue, background bootstrap, latency |
| 11 | [Decisions log](11-decisions-log.md) | Quick-reference table of major choices |
| 12 | [Routing accuracy (USP)](12-routing-accuracy.md) | Recall@3, golden suites, hybrid retrieval, feedback loop |
| 13 | [Proofs and benchmarks](13-proofs-and-benchmarks.md) | CI gates, `latest.json`, claim types (proven vs aspirational) |

## Relationship to other docs

| Document | Role |
|----------|------|
| [README.md](../../README.md) | Product overview, setup, feature matrix |
| [docs/benchmarks/](../benchmarks/README.md) | Published CI proof artifacts (`latest.json`) |
| [USAGE.md](../USAGE.md) | Operator guide, day-to-day commands |
| [host-integration.md](../host-integration.md) | Per-editor MCP config paths |
| [agent-brain/docs/concurrency.md](../../agent-brain/docs/concurrency.md) | Write-queue implementation detail |
| [CHANGELOG.md](../../CHANGELOG.md) | What shipped in each release |

## Shipped vs planned

The original [master spec](../superpowers/specs/mcp_router_master_spec.md) split **Phase 1** (local router) from **Phase 2+** (sync, federation, automation). As of v0.11, most Phase 2 items are **shipped** in some form; the series calls out where implementation diverged from the early draft spec.

## Contributing to this series

When you add a feature with non-obvious design choices:

1. Add a short **Alternatives considered** subsection to the relevant article, or
2. Add a row to [11-decisions-log.md](11-decisions-log.md)

Keep examples tied to real modules (`engine.rs`, `store.rs`, `route_gate.py`, etc.) so readers can verify claims in the repo.
