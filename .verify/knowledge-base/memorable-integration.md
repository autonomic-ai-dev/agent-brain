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

Before anything is written, each trace is reduced to its prompt (up to 2000 characters), a task description (up to 200), the harness, tool name, one target argument (`file_path`, `path`, `command`, `pattern`, or `url`, capped at 200 characters), and `exit_code` when known. Credential-shaped values are matched at token starts (`sk-`, `ghp_`, `AKIA`, `xox`) or as substrings (`-----BEGIN`, `Bearer `), so words like `task-` do not count. A payload with such a value is withheld, and the other payloads are still sent. The summary lists withheld payloads by number and marker. Each payload is written to `memorable/outbox.json` under the brain home and passed to `memorable ingest`, one workflow at a time. The file is removed after each send.

`capture-transcript` (`src/memorable.rs` `workflows_from_transcript`) reads a Claude Code `.jsonl` session. A human prompt starts a workflow. Meta records and local command messages (content starting with `<`) are skipped, as are workflows with no tool calls. Tool calls attach to the most recent workflow from the same repo. Records from sibling paths that share a prefix, such as `/repo2` for `/repo`, are excluded, because the match requires the exact repo path or a path below it.

## Known limits

- Capture needs the `memorable` binary on PATH and Memorable credentials (`memorable login`). Without them the service refuses the trace.
- Strength gate (`is_strong_workflow`, `MIN_TOOL_CALLS` = 3, `VERIFY_COMMANDS`): the client only sends workflows with a passing verifier run. Live result (2026-10-31, before the gate): `capture-transcript` on this repo's 11 sessions sent 27 workflows from the largest one. After the gate, the service still refused the strong ones with `no_postcondition`, so a passing verifier with an explicit `ok` is not enough. Capture stops at the first refusal and does not continue to later workflows.
- Checked against the Memorable CLI reference (https://www.memorable.sh/doc/cli, fetched 2026-10-31). Documented: consent gate; empty or read-only sessions are refused; refusals are logged to `~/.memorable/rejected.jsonl`; ingest JSON fields `session_id`, `task_description`, `harness`, `tool_calls` (`{name, input, result?}`), with `result` optional and sent only when the outcome is known. Not documented: minimum step count, postconditions, prefilter, `exit_code`, or whether `task_description` and `harness` are required. The `no_postcondition` refusal therefore has no documented cause. Check `~/.memorable/rejected.jsonl` for the reason the service logged, if it logs one for ingest refusals.
- Root cause found (2026-10-31): synthetic probes isolated the refusal. A workflow with `input: {target: ...}` was refused. The same steps with real keys (`file_path`, `command`) were stored, with or without `passed`, `precondition`, or `postcondition` fields. The reducer now keeps the real key. After the fix, 11 repo transcripts stored about 47 procedures in total. Some workflows were still refused, and the `-----BEGIN` marker withheld payloads that quote this code. The pasted note's `passed` field and `/v1/extract` requirement were not needed: the client has the `v1/extract` string but no `passed` field.
- Earlier live result (2026-10-31): the service admitted none. Each refusal was `no_postcondition`, `prefilter`, or `too_few_steps`. The postcondition rule is server-side and not visible in the client. Transcript outcomes (`is_error`) did not satisfy it. Nothing was stored, and the allowance was 0 of 1,000. Further sending waits on that rule.
- The reduction is a heuristic. A `command` target can still contain sensitive text, which is why capture stays opt-in.
- Memorable's own consent setting (`memorable enable`, run in its own CLI) is separate. This feature doesn't set it.

## Related

- Feature map: [[../features/memorable-integration]]
- Upstream MCP servers: [[cli-verify-and-memory]]
