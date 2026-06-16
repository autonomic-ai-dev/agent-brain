# Concurrency model

agent-brain uses a **single-threaded write queue** so all mutations to `brain.db` serialize through one path. Reads (`route_task`, `list_memory`, etc.) use the shared `BrainStore` mutex and may run concurrently with the queue, but never interleave writes.

## Queued operations (v0.7.2+)

| Operation | Entry point |
|-----------|-------------|
| `store_memory` | MCP tool |
| `delete_memory` | MCP tool |
| `import_memory` | MCP tool |
| `import` CLI | `agent-brain import <bundle>` |
| Git sync pull | `agent-brain sync git pull` |
| Cloud sync pull | `agent-brain sync cloud pull` |

All of the above enqueue `WriteOp` on the `Engine` write queue (spawned at `Engine::new` / MCP `serve`).

## Not queued

- **Reads:** `route_task`, `get_context`, `list_memory`, `explain_last_context`, bootstrap index scan
- **Exports:** `export`, `sync git push`, `sync cloud push` (read-only export + external I/O)
- **Session ingest** during bootstrap (background thread; rare overlap with MCP writes)

## Why it matters

Without the queue, a `sync cloud pull` during an active `store_memory` could interleave SQLite transactions and corrupt conflict resolution. Serializing imports and memory writes prevents that class of bug.

## Implementation

- `db/write_queue.rs` — channel + `WriteOp` enum
- `db/write_handler.rs` — executes ops on the writer thread
- `engine.rs` — owns `WriteQueue`; `import_bundle_queued()` for sync paths

## CLI / MCP approval (separate concern)

MCP **approval prompts** are controlled by Cursor (`~/.cursor/permissions.json`). See `agent-brain install --global`, which adds `agent-brain:*` to `mcpAllowlist`.
