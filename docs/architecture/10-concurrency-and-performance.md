# 10. Concurrency, performance, and reliability

## Summary

agent-brain optimizes for **low p95 route latency** on warm paths and **safe writes** under sync/import load. Reads are parallel-friendly; mutations serialize through one queue.

## What we built

### Write queue (`db/write_queue.rs`, `write_handler.rs`)

Queued operations:

- `store_memory`, `delete_memory`, `import_memory`
- CLI import, git pull, cloud pull

**Not queued:** `route_task`, `get_context`, exports, push operations (read-mostly + external I/O).

**Why:** Prevents interleaved SQLite transactions when sync pull races agent `store_memory` — class of bugs that manifest as subtle corruption or lost conflicts.

### Read path concurrency

- `BrainStore` behind `Mutex` — multiple MCP requests can read; SQLite read transactions are short
- **Search index cache** — in-memory snapshot of indexed rows + memories, invalidated on index bump
- **BM25 thread** — spawned parallel to embedding in `route_query_parallel`

### Caches

| Cache | Key | TTL / size |
|-------|-----|------------|
| Turn cache | scope + phase + query fp + index version | `turn_ttl_secs` (60s) |
| Query embedding | content hash | LRU 128 + SQLite table |
| ONNX model | filesystem | `~/.agent_brain/cache/fastembed` |

### Background bootstrap

Env-controlled delays so MCP handshake is not blocked:

- `AGENT_BRAIN_BOOTSTRAP_DELAY_SEC`
- `AGENT_BRAIN_BOOTSTRAP_INTERVAL_SEC`
- `AGENT_BRAIN_SESSION_INGEST_DELAY_SEC`

### Prewarm

Optional embed probe + search cache prewarm on bootstrap (`prewarm_on_bootstrap`).

### Auto-update (`auto_update.rs`)

While `serve` runs, periodic check for new GitHub release or package updates; downloads to staging and applies binary + refreshes `mcp.json` only when MCP is idle (via `mcp_activity`). Deferral window avoids killing active routes mid-turn.

### Multi-serve exclusion (`serve_lock.rs`)

Advisory lock on `<home>/serve.lock` prevents multiple `agent-brain serve` processes from corrupting SQLite and hook state concurrently. Multiple servers on the same home caused mid-turn "Tool execution error" and route gate flakes. Override with `AGENT_BRAIN_ALLOW_MULTI_SERVE=1`.

### Durable memory queue and CLI fallback (`memory_pending.rs`)

When `store_memory` fails due to client/socket transport errors, memories can be enqueued to `<home>/pending/` via `agent-brain memory enqueue` or written synchronously via `agent-brain memory put`. `agent-brain memory flush-pending` drains the queue with exponential backoff and retry limits.

### macOS codesign

Linker-signed local `cargo build` binaries are killed by **taskgated** when Cursor spawns MCP. Release CI Developer ID–signs `aarch64-apple-darwin`; install/doctor only verify and clear quarantine. Local builds: `make release-macos`.

### Resilience metrics

`RouteLatencyStats` tracks p95; logged per route. `serve_meta` compares running PID version vs disk binary (stale serve detection).

## Performance targets vs reality

| Target | Status | Proof |
|--------|--------|-------|
| Recall@3 ≥ 0.85 (memory + skills) | **CI-gated** | `proofs --ci` on isolated fixture |
| Turn-cache p95 ≤ 30 ms (500-skill fixture) | **CI-gated** | `proofs --ci`, deterministic embedder |
| Warm-route p95 ≤ 100 ms (fixture) | **CI-gated** | same |
| p95 < 50 ms route (real ONNX, full index) | **Design target** | `retrieval_log`, not CI |
| Cold first embed | seconds (install) | not gated |
| Index 500 items < 2s | informal | not gated |

CI gates these fixture budgets via `agent-brain proofs --ci` (not a product claim for ONNX + full index).

**Why not chase 5ms on cold path:** Correctness and local-first matter more than cold-start heroics; turn cache handles chatty follow-ups.

## Alternatives considered

### RwLock instead of Mutex for store

**Deferred:** Write volume low; Mutex simpler; snapshot cache reduces read contention.

### Separate indexer process

**Rejected:** IPC overhead, second binary to install.

### mmap embedding matrix

**Considered for scale:** Not needed at current index sizes.

### Unlimited parallel SQLite writers

**Rejected:** SQLITE_BUSY flakes and corruption risk.

## Trade-offs

- Write queue adds milliseconds to `store_memory` latency — acceptable for task-end writes.
- Turn cache can hide fresh index for up to TTL — index version in key limits staleness across reindex.
- Background threads + MCP single-threaded stdio — embed work blocks scoring thread briefly; parallel BM25 helps.

## For senior engineers and principal architects

### Read/write asymmetry by design

| Path | Concurrency | Rationale |
|------|-------------|-----------|
| `route_task` | Parallel reads, snapshot cache | Hot path; must not block on imports |
| `store_memory`, sync import | Single write queue | SQLite single-writer + conflict safety |

This is classic **read-heavy OLTP** shaping. If write volume grew 100×, we would batch writes or shard brains — neither is a solo-dev scenario today.

### Parallel BM25 + embed

`route_query_parallel` spawns BM25 on a thread while preparing embed — hides FTS latency behind ONNX when possible. Embedding still dominates on cache miss; query embedding LRU + SQLite `query_embeddings` amortize repeat phrasing within a session.

### Auto-update and `exec` restart

Auto-update replaces the binary when idle so **long-running `serve` does not pin old code**. Cursor reconnects MCP after restart. Trade-off: brief MCP blip — acceptable vs manual update friction for non-technical users.

### macOS taskgated (production lesson)

Development `cargo build` binaries lack proper signing → **silent kill** when Cursor spawns MCP. This looked like “MCP broken” not “codesign.” `make release-macos` and GitHub release binaries exist because **platform security policy is part of the architecture**, not ops trivia.

### SLO framing for PEs

| Metric | Target | Measurement | CI proof? |
|--------|--------|-------------|-----------|
| Recall@3 (fixture) | ≥ 0.85 | `proofs --ci` | **Yes** |
| Turn-cache p95 (fixture) | ≤ 30 ms | `proofs --ci` | **Yes** |
| Warm-route p95 (fixture) | ≤ 100 ms | `proofs --ci` | **Yes** |
| Warm route p95 (production ONNX) | <50 ms aspirational | `RouteLatencyStats`, `retrieval_log` | No |
| Cold first embed | seconds (install) | MCP logs | No |
| Write queue delay | <100 ms typical | not gated | No |

Do not promise sub-50ms on **first query after cold boot** — that missets expectations.

### Failure modes

| Symptom | Cause | Fix |
|---------|-------|-----|
| SQLITE_BUSY spikes | Parallel import + store | Serialize via queue (expected) |
| Memory growth | `retrieval_log` unbounded | Operator retention policy |
| Stale serve binary | Auto-update disabled | `serve_meta` version check |
| ONNX lock in tests | Parallel fastembed | Deterministic embedder in tests |

### Questions a PE should ask

1. Is **single Mutex** on store acceptable at your expected QPS? (Yes for IDE agent.)
2. What **retention** for logs and archived facts?
3. How do you **distribute signed binaries** to developers on macOS?
4. Do background bootstrap threads conflict with **corporate CPU policies**?

## Further reading

- [concurrency.md](../../agent-brain/docs/concurrency.md)
- [cache.rs](../../agent-brain/src/cache.rs)
- [db/latency.rs](../../agent-brain/src/db/latency.rs)
