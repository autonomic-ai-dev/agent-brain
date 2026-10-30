---
feature: "[[../features/memorable-integration]]"
---

# Memorable integration

## What it does

Memorable is a procedural-memory CLI. Its MCP server reads procedures that are stored on the machine and never writes. This feature makes that server available to agents, behind an explicit opt-in flag that defaults to off.

## How it works

1. `agent-brain memorable enable` sets `memorable.enabled: true` in the brain config and registers an upstream MCP server named `memorable`. The server runs `memorable mcp` and gets `MEMORABLE_API_URL=http://127.0.0.1:9`.
2. The registered server's tools (`memorable_recall`, `memorable_list`, `memorable_show`, `memorable_status`, `memorable_explain_recall`) are reached through `route_to_mcp`, like any other upstream server.
3. `agent-brain memorable disable` removes the server and clears the flag.

## Where the code is

- `agent-brain/src/memorable.rs`: `enable`, `disable`, and `is_registered`, with unit tests
- `agent-brain/src/settings.rs`: the `memorable` section, defaulting to `enabled: false`
- `agent-brain/src/main.rs`: the `memorable` command

## How it is verified

- `cargo test -p agent-brain --lib memorable`
- Sandbox run: `AUTONOMIC_HOME` pointed at a scratch directory, `status`, `enable`, `status`, `disable`, `status` (the config changed as expected)
- Phase 1 and 2 egress tests (sandbox, listener on localhost): `status`, `doctor`, `enable`, `recall`, `install-hooks`, the prompt hook, and the MCP server made 0 connections. `ingest` was refused before any connection, because it needs credentials.

## Capture (opt-in)

Capture sends traces to Memorable's extraction service, so it has two gates on top of the read-only link:

1. `memorable.capture` is set by `agent-brain memorable enable --capture`. It is off by default.
2. The last `agent-brain verify` for the repo must have passed (`verify::last_verification`).

Before anything is written, the trace is reduced to tool name, one target argument (`file_path`, `path`, `command`, `pattern`, or `url`, capped at 200 characters), and `exit_code`. A credential-shaped value anywhere in the reduced trace stops the capture. The reduced trace is written to `memorable/outbox.json` under the brain home and passed to `memorable ingest`. The file is removed afterwards.

## Known limits

- Capture needs the `memorable` binary on PATH and Memorable credentials (`memorable login`). Without them the service refuses the trace.
- The reduction is a heuristic. A `command` target can still contain sensitive text, which is why capture stays opt-in.
- Memorable's own consent setting (`memorable enable`, run in its own CLI) is separate. This feature doesn't set it.

## Related

- Feature map: [[../features/memorable-integration]]
- Upstream MCP servers: [[cli-verify-and-memory]]
