---
feature: "[[../features/serve-daemon-and-proxy]]"
---

# Serve daemon and stdio proxy

## What it does

Every agent host session starts `agent-brain serve` as an MCP server. One of those processes is a stdio proxy. It forwards the host's requests to a shared daemon over a Unix socket. The daemon owns the brain database and the embedding model, and it runs background work once for all sessions.

## How it works

1. **Daemon.** `agent-brain serve --daemon` takes the serve lock, starts bootstrap and auto-update, writes `serve_meta`, runs auto-wire, and listens on the daemon socket. It also starts a check every 60 seconds that drops the embedding model after 10 minutes without use.
2. **Proxy.** `agent-brain serve` without `--daemon` connects to the socket. If no daemon answers, it starts one and retries. After a daemon restart it replays the `initialize` handshake so the host session continues.
3. **Exit.** A proxy exits when its host closes stdin, and at each reconnect it exits if its parent process has changed. The daemon keeps running for other sessions.
4. **Embedder.** The model loads on the first embed and stays resident until the idle check unloads it. The next embed reloads it, which takes a moment.

## Where the code is

- `agent-brain/src/main.rs`: the `serve` branch, which chooses daemon or proxy.
- `agent-brain/src/mcp/server.rs`: `run_proxy` and `serve_one_proxy_connection`.
- `agent-brain/src/engine.rs`: `spawn_embedder_idle_unload`.
- `agent-brain/src/embed.rs`: `Embedder::unload_if_idle`, `is_loaded`, and `IDLE_UNLOAD_AFTER`.
- `agent-brain/tests/proxy_lifecycle.rs`: the regression test for proxy exit.
- The feature map is [[../features/serve-daemon-and-proxy]].
