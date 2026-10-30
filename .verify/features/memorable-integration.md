---
feature: memorable-integration
knowledge_base: "[[../knowledge-base/memorable-integration]]"
related:
  - "[[cli-verify-and-memory]]"
---

# memorable-integration

Summary: An opt-in, read-only link to the Memorable procedural-memory CLI. It is off by default. When enabled it registers Memorable's MCP server as an upstream and points that server at an unreachable API, so the server cannot send anything out. Sending traces to Memorable's extraction service is not part of this feature.

## Sub-features

- `memorable-toggle`: Turns the read-only link on and off.
  - Behaviour: `agent-brain memorable status | enable | disable`. `enable` sets `memorable.enabled: true` and registers an upstream server named `memorable` (`memorable mcp`, with `MEMORABLE_API_URL=http://127.0.0.1:9`). `disable` removes the server and clears the flag.
  - Outcome: The flag and the server registration change together. Other upstream servers are left as they are.

- `memorable-capture`: Sends verified sessions to Memorable's extraction service. Off by default, and needs its own flag.
  - Behaviour: `agent-brain memorable enable --capture` sets `memorable.capture: true`. `agent-brain memorable capture <trace.json>` then requires the flag, a passing last `agent-brain verify` for the repo, and no credential-shaped values. It reduces the trace to tool name, one target argument, and exit code, writes it to `memorable/outbox.json` under the brain home, and runs `memorable ingest` on it.
  - Outcome: Nothing is sent when the flag is off or the last verify failed. The trace the service receives carries no prompt, no transcript, and no tool output.

## Code

- `agent-brain/src/memorable.rs`
- `agent-brain/src/settings.rs` (the `memorable` section and its default)
- `agent-brain/src/mcp_cli.rs` (`persist_brain_settings`, now public)
- `agent-brain/src/main.rs` (the `memorable` command arm)

## How to get to it (user POV)

- `agent-brain memorable status`, then `agent-brain memorable enable` and `agent-brain mcp sync`

## Verification Suites (Targeted Test Commands)

- `cargo test -p agent-brain --lib memorable` (defaults off, enable registers the blackholed read-only server, enable is idempotent, disable keeps other servers)

## Gotchas

- Memorable's `ingest` and `record` send traces to its extraction API. Nothing in this feature calls them, and the `memorable enable` output says so.
- The `memorable` binary must be on PATH for the server to start. Otherwise `mcp sync` fails for that server.
- The blackholed API URL stops the server from reaching the cloud even if credentials are added later. Its recall is then exact and lexical only.
