---
feature: serve-daemon-and-proxy
knowledge_base: "[[../knowledge-base/serve-daemon-and-proxy]]"
related:
  - "[[cli-verify-and-memory]]"
---

# serve-daemon-and-proxy

Summary: `tekmor serve` runs in two roles. A shared daemon holds the brain database, the embedding model, and background work on a Unix socket. Each host session starts a stdio proxy that bridges the host to that socket. This map covers the daemon, the proxy lifecycle, and the embedder's idle unload.

## Sub-features

- `daemon-serve`: Runs the shared daemon for all host sessions.
  - Intent: One process owns the database and the model, so sessions do not each load them.
  - Behaviour: `tekmor serve --daemon` takes the serve lock, runs bootstrap and auto-update, writes `serve_meta`, runs auto-wire, and serves the daemon socket. It also starts the embedder idle check (`Engine::spawn_embedder_idle_unload`).
  - Outcome: One daemon per home. A second start fails on the lock.
- `proxy-stdio-bridge`: Bridges a host session's stdio to the daemon socket.
  - Intent: Let each host start its MCP server without owning the daemon.
  - Behaviour: `tekmor serve` without `--daemon` (`run_proxy` in `tekmor/src/mcp/server.rs`) connects to `serve.sock`. If no daemon answers, it starts `serve --daemon` once, then retries. After a daemon restart it replays the `initialize` handshake.
  - Outcome: Requests reach the daemon, and a daemon restart does not end the host session.
- `proxy-exits-with-host`: Stops a proxy when its host has gone.
  - Intent: A closed host must not leave a proxy running. Each one holds a process and a socket connection.
  - Behaviour: The proxy returns when the host closes its stdin. At each reconnect it also compares its parent process ID with the one it started under, and returns if they differ.
  - Outcome: The proxy process exits. The daemon keeps running, because other sessions may use it.
- `embedder-idle-unload`: Drops the embedding model after a period without use.
  - Intent: The model is about 180 MB of resident memory. Keep it only while queries use it.
  - Behaviour: The model loads on the first embed. A check every 60 seconds drops it once it has been unused for `IDLE_UNLOAD_AFTER` (600 seconds, `src/embed.rs`). The next embed reloads it.
  - Outcome: The daemon's footprint falls back after idle time. The first query after idle pays the model load.

- `mcp-list-tools-filter`: The MCP tool list honours `mcp.tool_surface`.
  - Behaviour: `list_tools` in `src/mcp/server.rs` overrides the `#[tool_handler]` default, which the rmcp macro emits only when the method is absent. The route handler in the same file now also sets `file_context` and `lessons` on the response.
  - Outcome: Hidden tools are still callable by name, and the tool list shrinks only when the setting is `core`.

- **Tool surface filter:** `visible_tools` in `src/mcp/server.rs` applies `mcp.tool_surface` to `list_tools`. Hidden tools stay in the router, so a call by name still works.
  - Outcome: Tests `the_core_surface_lists_exactly_the_core_tools` and `the_full_surface_lists_every_routed_tool_and_hidden_tools_stay_routed`.

- `mcp-core-route-response`: Under `mcp.tool_surface: core`, the MCP route response omits `context_bundle` (R-8).
  - Behaviour: `core_tool_surface` in `tekmor/src/mcp/server.rs` clears `context_bundle` before `route_task` serializes the response. The gRPC response keeps it.
  - Outcome: Core-surface agents get a smaller route response. Full-surface agents see no change.

- `mcp-once-per-session`: The MCP route response sends static text once per session (R-6, R-16).
  - Behaviour: `once_per_session` in `tekmor/src/mcp/server.rs` sends each warning topic and the native-tool list once per connection. Later routes omit them.
  - Outcome: The contract and native-tool text are not repeated on every turn.

## Code

- `tekmor/src/main.rs` (`serve` branch)
- `tekmor/src/mcp/server.rs` (`run_proxy`, `serve_one_proxy_connection`)
- `tekmor/src/engine.rs` (`spawn_embedder_idle_unload`)
- `tekmor/src/embed.rs` (`Embedder::unload_if_idle`, `is_loaded`, `IDLE_UNLOAD_AFTER`)
- `tekmor/src/daemon_watchdog.rs` (`DaemonActivity` implements `Default`, delegating to `new`)
- `tekmor/tests/proxy_lifecycle.rs`

## How to get to it (user POV)

- CLI: `tekmor serve --daemon` (shared daemon), `tekmor serve` (proxy, started by the host)
- Host config: the host's MCP entry runs `tekmor serve`, so the proxy starts with each session

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test --all-features embed::tests::` (6 tests, embedder loading and unload)
- **Integration Test:** `cargo test --all-features --test proxy_lifecycle` (1 test, proxy exit with its host)
- **Full suite:** `cargo test --all-features -- --skip graphify_1k_ci_gate --test-threads=1` (the serial gate; see `cli-verify-and-memory.md`)
- **Functional tests:** not yet written for the daemon socket. The integration test above covers the proxy's exit with its host. Add a functional suite here when the daemon socket gains a documented surface.
- **Preconditions:** Rust toolchain. The embedder tests load the ONNX model from the default cache directory, so they need the model cached or network access on first run. The proxy test binds a listener in a temporary `TEKMOR_HOME`, so it never reaches the real daemon.

### Verification Steps

1. **proxy-exits-with-host-test** (sub-feature: proxy-exits-with-host)
   - Request: `cargo test --all-features --test proxy_lifecycle`
   - Why: Proves a proxy whose host exits does not keep running.
   - Expected: 1 passed, 0 failed (verified in this pass, `.verify/artifacts/brain-proxy-lifecycle.log`).
   - Impact: A failure means the orphaned proxy regression is back. Each leftover proxy holds a process and a socket connection.
2. **embedder-unload-tests** (sub-feature: embedder-idle-unload)
   - Request: `cargo test --all-features embed::tests::`
   - Why: Proves the model unloads after idle, reloads on next use, and stays loaded while recently used.
   - Expected: 6 passed, 0 failed (verified in this pass, `.verify/artifacts/brain-embed.log`). The unload tests are `idle_unload_drops_model_and_next_use_reloads_it` and `recently_used_model_stays_loaded`.
   - Impact: A failure means the daemon's memory can grow without bound, or the first query after idle fails.

## Gotchas

- The MCP graph tools in `mcp/server.rs` (`query_codebase`, `trigger_deep_analysis`, `graphify_job_status`) call the graphify extension through `graphify::rpc`. They no longer run the graph engine in the serve process, so they need the extension installed.

- The daemon is shared and outlives the proxy that started it. It is not an orphan, and nothing ends it when its last proxy exits.
- A proxy captures its parent PID when it starts. If the host exits before the proxy captures it, the proxy records the new parent (PID 1) and the parent check never fires. The host-stdin path still ends the proxy once a daemon connection exists. A proxy that never connects to a daemon, and whose host dies before it starts, is still a known gap.
- The proxy test binds a listener that never accepts, so the proxy connects without starting a daemon. Do not point `TEKMOR_HOME` at the real home in a test.
- Installed binaries keep the old behaviour until they are rebuilt and reinstalled from a merged `master`. Restart host sessions after reinstalling, or their old proxies keep running.

- Formatting pass (rustfmt, 2026-11-07): line breaks and spacing only, no behaviour change.
- Session-scoped route state (2026-11-13): each MCP session gets its own `BrainMcp`, so the record of constraints already sent (`src/mcp/sent_constraints.rs`) is per host session even when sessions share one daemon. `src/engine.rs` returns a rule once when the same topic and text are indexed from several paths.
