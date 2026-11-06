---
feature: "[[../features/graphify-extension]]"
---

# Graphify extension

## What it does

The codebase graph moved out of the core binary into the `agent-brain-graphify` extension. Graph ingest, background graph builds, repo registration, the graph CLI, and the heavy graph queries run in the extension. Core keeps the cheap route read path, so route responses still carry code context without starting the extension.

## How it works

1. **Route path (core, in process).** `route_task` calls `graphify::route_code_context` (`agent-brain/src/graphify/mod.rs`). It reads graph tables through `BrainStore` accessors (`agent-brain/src/db/graph_store.rs`). No extension process runs.
2. **MCP graph tools (core server, out of process).** `query_codebase`, `trigger_deep_analysis`, and `graphify_job_status` keep their names and route gate in `agent-brain/src/mcp/server.rs`. Each calls `graphify::rpc`, which spawns `<brain home>/extensions/bin/agent-brain-graphify rpc <tool> <json>`. The extension prints one JSON line on stdout, and core reads that line.
3. **Background builds.** `trigger_deep_analysis` prints the job id first, then the extension keeps running the build on a thread until it finishes. Core reaps the child on a background thread and does not wait.
4. **CLI.** `agent-brain graphify <sub>` routes to the extension binary, which runs `run_cli`.

## Where the code is

- Extension engine: `extensions/graphify/src/` (`ingest.rs`, `jobs.rs`, `repos.rs`, `query.rs`, `process.rs`, `cli.rs`, `types.rs`, `rpc.rs`, `main.rs`, `lib.rs`)
- Core route path: `agent-brain/src/graphify/mod.rs` (`route_code_context`, `rpc`), `agent-brain/src/graphify/rows.rs`
- Core store accessors: `agent-brain/src/db/graph_store.rs` (`insert_graphify_job` takes a `GraphifyJobInsert` struct; the extension's `jobs.rs` builds one per queued job)
- Core MCP handlers: `agent-brain/src/mcp/server.rs` (`query_codebase`, `trigger_deep_analysis`, `graphify_job_status`)
- Bench consumer: `extensions/bench/src/graphify_bench.rs` (depends on the `agent_brain_graphify` library)

## How it is verified

- `cargo build -p agent-brain-graphify`
- `cargo test -p agent-brain-graphify` (in the allowlist)
- `./target/debug/agent-brain-graphify graphify status` prints a JSON list
- `./target/debug/agent-brain-graphify rpc graphify_job_status '{"job_id":"none"}'` prints an error

## Known limits

- The extension must be installed for the graph MCP tools to work. Without it, the tools return an error that names `ext add graphify`.
- Each graph MCP call starts the extension process and opens the store, so it costs more than an in-process call. Route code context does not pay this cost, because it stays in core.
- Extension logs go to its stderr, which the core server passes through to the host, not to the core log file.
- The extension's `rpc` protocol assumes a single JSON line on stdout. Tool output that adds lines breaks the core reader.

## Related

- Feature map: [[../features/graphify-extension]]
- Extension mechanism: [[cli-verify-and-memory]]
- Bench consumer: [[bench-extension]]
