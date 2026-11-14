---
feature: graphify-extension
knowledge_base: "[[../knowledge-base/graphify-extension]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[bench-extension]]"
  - "[[extras-extension]]"
---

# graphify-extension

Summary: The codebase graph engine, shipped as the optional `tekmor-graphify` extension. The extension owns graph ingest, background graph builds, repo registration, the `graphify` CLI, and the heavy graph queries. Core keeps only the cheap read path that adds code context to routes, plus the row types and the store accessors it reads, so routing stays fast without the extension.

## Sub-features

- `graphify-command`: The codebase graph CLI.
  - Behaviour: `tekmor graphify <enable|disable|status|ingest|run|query> ...`. The extension runs the subcommand against the core store.
  - Outcome: The same output as before the move.
- `graphify-query-tool`: The `query_codebase` MCP tool, served on demand.
  - Behaviour: The core MCP server keeps the tool name and its route gate. It calls the installed extension with `tekmor-graphify rpc query_codebase <json>`, which prints one JSON line with the answer.
  - Outcome: The same answer as before. The first call pays the extension's startup cost.
- `graphify-job-tools`: The `trigger_deep_analysis` and `graphify_job_status` MCP tools.
  - Behaviour: The core server calls `tekmor-graphify rpc trigger_deep_analysis <json>`, which prints the job id and keeps running the build until it finishes. The core server reads only the first line and does not wait. `graphify_job_status` reads the job row through the extension.
  - Outcome: A queued build survives after the core call returns.
- `graphify-route-context`: Graph context in route responses (stays in core).
  - Behaviour: `route_code_context` reads the graph tables through core store accessors. No extension process runs on the route path.
  - Outcome: Route code context works without the extension installed. Graph data is only refreshed by the extension.

## Code

- `extensions/graphify`
- `tekmor/src/graphify/mod.rs`
- `tekmor/src/graphify/rows.rs`
- `tekmor/src/db/graph_store.rs`
- `Cargo.toml`

## How to get to it (user POV)

- `tekmor ext add graphify`, then `tekmor graphify status`
- Graph MCP tools need the extension installed. Without it they return an error that names the install command.
- Developers can run `./target/debug/tekmor-graphify graphify status` after `cargo build -p tekmor-graphify`.

## Verification Suites (Targeted Test Commands)

- **Build:** `cargo build -p tekmor-graphify`
- **Smoke:** `./target/debug/tekmor-graphify graphify status` prints a JSON list; `./target/debug/tekmor-graphify rpc graphify_job_status '{"job_id":"none"}'` prints an error line.
- **Allowlisted:** `cargo test -p tekmor-graphify` (in `.verify/verify.toml`).

## Gotchas

- Core keeps the `AstCodeNodeRow`, `AstEdgeRow`, `CodeGraphNodeRow`, `CodeGraphEdgeRow`, and `LocalGraphEdge` types, because `index.rs` writes AST rows and a core test uses them.
- Core keeps the `BrainStore` graph accessors in `tekmor/src/db/graph_store.rs`, because route and index code call them. `GraphifyJobRecord` lives there and is re-exported by the extension.
- `rpc` is a single-line JSON protocol. Any extra stdout output from a tool breaks the core reader.
- The extension must be rebuilt with the core, since it links the core library.
- Graph MCP calls now run outside the core process, so their logs are not in the core log.
- Job rows are written through `BrainStore::insert_graphify_job(GraphifyJobInsert { .. })` in `tekmor/src/db/graph_store.rs`. The extension's `jobs.rs` queues a job with status `queued`, and the struct replaces the old nine-argument call.

- Formatting pass (rustfmt, 2026-11-07): line breaks and spacing only, no behaviour change.
