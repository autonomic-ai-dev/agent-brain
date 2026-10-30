---
feature: serve-daemon-and-proxy
knowledge_base: "[[../knowledge-base/serve-daemon-and-proxy]]"
related:
  - "[[cli-verify-and-memory]]"
---

# serve-daemon-and-proxy

Summary: `agent-brain serve` runs in two roles. A shared daemon holds the brain database, the embedding model, and background work on a Unix socket. Each host session starts a stdio proxy that bridges the host to that socket. This map covers the daemon, the proxy lifecycle, and the embedder's idle unload.

## Sub-features

- `daemon-serve`: Runs the shared daemon for all host sessions.
  - Intent: One process owns the database and the model, so sessions do not each load them.
  - Behaviour: `agent-brain serve --daemon` takes the serve lock, runs bootstrap and auto-update, writes `serve_meta`, runs auto-wire, and serves the daemon socket. It also starts the embedder idle check (`Engine::spawn_embedder_idle_unload`).
  - Outcome: One daemon per home. A second start fails on the lock.
- `proxy-stdio-bridge`: Bridges a host session's stdio to the daemon socket.
  - Intent: Let each host start its MCP server without owning the daemon.
  - Behaviour: `agent-brain serve` without `--daemon` (`run_proxy` in `agent-brain/src/mcp/server.rs`) connects to `serve.sock`. If no daemon answers, it starts `serve --daemon` once, then retries. After a daemon restart it replays the `initialize` handshake.
  - Outcome: Requests reach the daemon, and a daemon restart does not end the host session.
- `proxy-exits-with-host`: Stops a proxy when its host has gone.
  - Intent: A closed host must not leave a proxy running. Each one holds a process and a socket connection.
  - Behaviour: The proxy returns when the host closes its stdin. At each reconnect it also compares its parent process ID with the one it started under, and returns if they differ.
  - Outcome: The proxy process exits. The daemon keeps running, because other sessions may use it.
- `embedder-idle-unload`: Drops the embedding model after a period without use.
  - Intent: The model is about 180 MB of resident memory. Keep it only while queries use it.
  - Behaviour: The model loads on the first embed. A check every 60 seconds drops it once it has been unused for `IDLE_UNLOAD_AFTER` (600 seconds, `src/embed.rs`). The next embed reloads it.
  - Outcome: The daemon's footprint falls back after idle time. The first query after idle pays the model load.

## Code

- `agent-brain/src/main.rs` (`serve` branch)
- `agent-brain/src/mcp/server.rs` (`run_proxy`, `serve_one_proxy_connection`)
- `agent-brain/src/engine.rs` (`spawn_embedder_idle_unload`)
- `agent-brain/src/embed.rs` (`Embedder::unload_if_idle`, `is_loaded`, `IDLE_UNLOAD_AFTER`)
- `agent-brain/tests/proxy_lifecycle.rs`

## How to get to it (user POV)

- CLI: `agent-brain serve --daemon` (shared daemon), `agent-brain serve` (proxy, started by the host)
- Host config: the host's MCP entry runs `agent-brain serve`, so the proxy starts with each session

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test --all-features embed::tests::` (6 tests, embedder loading and unload)
- **Integration Test:** `cargo test --all-features --test proxy_lifecycle` (1 test, proxy exit with its host)
- **Full suite:** `cargo test --all-features -- --skip graphify_1k_ci_gate --test-threads=1` (the serial gate; see `cli-verify-and-memory.md`)
- **Functional tests:** not yet written for the daemon socket. The integration test above covers the proxy's exit with its host. Add a functional suite here when the daemon socket gains a documented surface.
- **Preconditions:** Rust toolchain. The embedder tests load the ONNX model from the default cache directory, so they need the model cached or network access on first run. The proxy test binds a listener in a temporary `AGENT_BRAIN_HOME`, so it never reaches the real daemon.

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

- The daemon is shared and outlives the proxy that started it. It is not an orphan, and nothing ends it when its last proxy exits.
- A proxy captures its parent PID when it starts. If the host exits before the proxy captures it, the proxy records the new parent (PID 1) and the parent check never fires. The host-stdin path still ends the proxy once a daemon connection exists. A proxy that never connects to a daemon, and whose host dies before it starts, is still a known gap.
- The proxy test binds a listener that never accepts, so the proxy connects without starting a daemon. Do not point `AGENT_BRAIN_HOME` at the real home in a test.
- Installed binaries keep the old behaviour until they are rebuilt and reinstalled from a merged `master`. Restart host sessions after reinstalling, or their old proxies keep running.
