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
  - Behaviour: `agent-brain memorable enable --capture` sets `memorable.capture: true`. `agent-brain memorable capture <trace.json>` sends one trace. `agent-brain memorable capture-transcript <session.jsonl>` splits a Claude Code transcript into one workflow per human prompt, limited to records whose `cwd` is the current repo or below it. Only strong workflows are sent: at least 3 tool calls, at least one state-changing call (`Edit`, `MultiEdit`, `Write`, or `NotebookEdit`), since the Memorable CLI reference (https://www.memorable.sh/doc/cli) says a session that only read and searched is not stored, and a Bash run of a verifier (`cargo test`, `cargo clippy`, `agent-brain verify`, `pytest`, `npm test`, or `make test`) with an explicit `ok: true`. Weak workflows are skipped and counted in the summary. Both require the flag, a passing last `agent-brain verify` for the repo, and withholds any payload that holds a credential-shaped value, reporting it by number. Each payload carries the prompt (up to 2000 characters), a task description (the first substantive line of the prompt, up to 200 characters; lines that start with `<` or `/` are skipped), both scrubbed of the home directory (shown as `~`) and of email addresses (shown as `<email>`), as the gbrain page describes the derivation, the harness, and the tool calls reduced to name and the argument the tool used (`file_path`, `path`, `command`, `pattern`, or `url`, under its real key, not renamed), capped at 200 characters. Renaming the key to `target` made the service refuse every workflow (`no_postcondition, prefilter`), while the same steps under real keys were stored. Each payload is written to `memorable/outbox.json` under the brain home and passed to `memorable ingest`.
  - Outcome: Nothing is sent when the flag is off or the last verify failed. A payload with a credential-shaped value is withheld, not sent. The service receives the prompt text and tool targets for each workflow. It does not receive tool output or the transcript. A tool result is sent only from an explicit `exit_code` or `ok` flag. Transcripts carry an `is_error` flag on about 58% of results, and only those flags are used. A missing flag is not guessed.

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
