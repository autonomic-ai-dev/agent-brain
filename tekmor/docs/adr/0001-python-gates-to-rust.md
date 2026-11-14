# ADR 0001: Move the host route, read and shell gates from Python to Rust

Status: Proposed. No code until the owner approves this note.

## Context

`tekmor/hooks/route_gate.py` is 1,379 lines in 78 functions. It runs on every host tool call. It holds three gates:

- the route gate, which blocks tool calls until `route_task` has run this turn (`gate_tool_use`, `session_route_reusable`, `load_state`, `save_state`);
- the read gate, which blocks large unscouted reads (`check_read_tool_gate`);
- the Python shell policy, which blocks Python used to edit files (`check_python_shell_policy`).

The command policy is already in Rust. `check_command_policy` runs `tekmor bash-gate`. The stop gate is in Rust too (`tekmor/src/stop_gate.rs`). So the Python file duplicates logic that exists in the binary, and the two copies can drift.

The plan (R-15) records 49 ms per call for the Python gate against 10 ms for the Rust gate. Those figures come from the plan's audit, not from a measurement made for this note. A benchmark is part of the acceptance criteria below.

## Decision

Add a Rust subcommand, `tekmor route-gate`. It reads the host event as JSON on stdin and writes the host decision as JSON on stdout. It runs the same three gates.

The Python file stays for one release as a thin caller: it reads stdin, runs `tekmor route-gate`, and prints the result. An environment switch, `TEKMOR_ROUTE_GATE_IMPL=python`, keeps the old path available during the transition.

## Scope

In scope: the route gate, the read gate, the Python shell policy, and the host event normalization they need.

Out of scope: the command policy (already `tekmor bash-gate`), the stop gate (already Rust), and the host installer.

## Contract

- Input: the host event JSON. Hosts send different shapes (Claude Code, Cursor, OpenCode, Antigravity, Gemini, Codex). Normalization stays one table in Rust, with one test per host shape.
- Output: `{"permission": "allow" | "deny", "agent_message": ..., "user_message": ...}`, the shape the Python hook already returns.
- State: the same file, `route_state.json`, with the same keys, so a session routed under Python is still routed under Rust. Keys are not renamed in this release.
- Failure: fail closed where the Python gate fails closed today. A missing binary denies, as `check_command_policy` does now.

## Migration

1. Write `tekmor route-gate` with unit tests. Use the 79 cases in `tekmor/hooks/test_route_gate.py` as the specification. Each Python test has a Rust counterpart or an explicit "not ported" note.
2. Shadow mode: the Python hook runs the Rust gate and logs any disagreement to the gate-event log. No decision changes.
3. Switch: the Python hook becomes the thin caller. `TEKMOR_ROUTE_GATE_IMPL=python` selects the old code.
4. Remove the Python gate logic after one release with no disagreements in the shadow log.

## Acceptance

- All 79 Python test cases pass against the Rust gate, or each gap is listed with a reason.
- Shadow mode records zero disagreements over one week of owner use.
- Warm-path p95 under 15 ms, measured with `tekmor bench` on the same machine as the Python baseline.

## Amendment 1: step 2 scope

Step 2 delivers the pure route decision in `tekmor/src/route_gate_decision.rs`. It takes the tool name, the route state and the current time, and returns allow, allow without route (with its reason), or deny. It covers route_task, discovery tools, the grace window, MCP offline mode and the stale-route timeout.

The `tekmor route-gate` subcommand and the reading and writing of `route_state.json` move to step 3, with shadow mode. Step 2 wires nothing into a host. The silent-watcher setting, the read gate, and the Python shell policy are not ported yet. Step 3 must port or explicitly exclude each before the switchover in step 4.

## Rollback

Set `TEKMOR_ROUTE_GATE_IMPL=python` to return to the Python gate. Reverting the commit restores the Python file. The state file format does not change, so no migration is needed.

## Risks

- Host event shapes differ and change between host releases. The normalization table is the most likely place for a regression.
- The Python gate runs in the host's Python. The Rust gate needs the `tekmor` binary on PATH. The hook must report a clear message when it is missing.
- Latency depends on process start. The release binary must be the one the hook calls.
