---
feature: "[[../features/cli-verify-and-memory]]"
---

# CLI, verify, and memory

## What it does

Version 1.0.0-rc5 (`agent-brain/Cargo.toml`). See CHANGELOG.md for the release contents.

agent-brain is the routing layer every agent calls at the start of a turn. Its CLI and MCP tools run a command and record its real exit code as evidence. They also store and retrieve durable memory, install themselves into hosts, and report their own health.

## How it works

1. **Verify.** `agent-brain verify [--source cli|orchestrator] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code, source, the summed `test result:` count (`tests_run`, schema v22), and `code_hash` (schema v24), the hash of the code tree the run tested. A run with no cargo summary stores no count.
2. **Harness scaffold.** `agent-brain verify init` writes `.verify/verify.toml`, an executable `run.sh`, and a README.
3. **Report.** `agent-brain verify record` and `verify report` summarize recorded exits per source.
4. **Memory.** `agent-brain memory put` writes a durable fact. `enqueue` and `flush-pending` hold facts that could not be written yet and retry them later. The MCP `store_memory` and `delete_memory` tools go through the write queue on `spawn_blocking`, so a slow write does not park an async worker.
5. **Route.** `route_task` ranks skills, rules, and memory for a turn under a token budget, applies the must-apply rules, and returns suggested tools. Ranking lives in the core crate (see [core-crate](core-crate.md)).
6. **Stop gate.** `agent-brain stop-gate` runs as the Stop hook. It blocks a stop when code changed in a repo and either the default verify step fails, or the feature map or knowledge base has not been updated (see below).
7. **Install and doctor.** `agent-brain install --global` writes MCP config, hooks, and permissions. `doctor` checks health, and `doctor --fix` clears quarantine.
8. **Sync.** `agent-brain sync git|cloud` and `bootstrap` move memory between machines.
9. **Feedback report.** Opt-in with `feedback_report.enabled` (default false, interval `interval_hours`, default 3). When a report is due, `route_task` writes `reports/telemetry-<ts>.md` under the brain home and adds a `feedback-report` must-apply item asking the agent to write `reports/agent-feedback-<ts>.md`. The item includes why agstack-mode was not applied, and it repeats until that file exists. `src/feedback_report.rs` holds the logic.
10. **Hook events.** `hooks/route_gate.py` appends each host hook event to `hooks/hook_events.jsonl` with its session ID. The feedback report reads this file.
11. **Feature graph.** `agent-brain feature-graph [DIR]` prints the feature maps as JSON nodes and edges, built from frontmatter and the `## Sub-features` and `## Code` sections. `src/feature_graph.rs` holds the logic.

### Stop gate rules

- A passing run counts only for the code tree it tested. The gate hashes the changed code paths (under `src/`, `crates/`, `tests/`, or `hooks/`, or a `Cargo.*` file, never under `.verify/`) and compares that hash with the `code_hash` stored on each verify run. Editing docs does not invalidate a run.
- Only the repo the hook runs in is checked. A workspace folder that is not a repo is not scanned for child repos.
- The stop-gate unit tests run with `cargo test -p agent-brain --lib stop_gate`, which is on the allowlist in `.verify/verify.toml` and can be run as a targeted check.
- A doc-only change runs no verify step. The feature-map and knowledge-base rules still apply to it.
- A repo with changed files needs a passing `agent-brain verify` run recorded after the last edit. Only `cli` and `orchestrator` sources count.
- If the repo has `.verify/features/`, a code change also needs a changed file under `.verify/features/` and one under `.verify/knowledge-base/`. A passing verify run does not lift this block.
- A block is repeated up to three times for the same edit. After that the stop is allowed, and the gate prints why.

## Where the code is

- CLI and verify: `agent-brain/src/verify.rs`, `count_tests` for the cargo count
- Stop gate: `agent-brain/src/stop_gate.rs` (`changed_paths`, `feature_map_missing`, `feature_map_gap`, `evaluate_stop`)
- Host hooks: `agent-brain/src/host_hooks.rs` (`claude_code_gate_fragment` installs the `Stop` hook)
- MCP tools: `route_task`, `store_memory`, `verify_run`, `record_verify_outcome`, `file_summary`, `grep_search`, `read_file_head`, `read_file_tail`
- Memory write path: `agent-brain/src/mcp/server.rs` (`store_memory`, `delete_memory`), `agent-brain/src/db/write_handler.rs` (`send_and_recv`), `agent-brain/src/db/store.rs` (`store_fact_full`)
- graphify subprocess timeouts: `agent-brain/src/graphify/process.rs` (`output_with_timeout`), used by `graphify/jobs.rs` and `graphify/query.rs`

## How to use it

- `agent-brain verify -- cargo test` to record a real result
- `agent-brain install --global` to install hooks for every host
- `agent-brain doctor` to check health

## How it is verified

- `cargo test --all-features` (about 300 unit tests plus every integration target)
- The stop gate tests in `stop_gate.rs`: the verification rules, the feature-map and knowledge-base rule, and the end-to-end cases with a real git repository
- `sh .verify/checks/core_boundary.sh` for the core crate's dependency boundary

## Behaviour that matters

- `grep_search` accepts `query` as an alias for `pattern`, and `path` defaults to `"."`. Callers that send `query` no longer fail with a missing-field error.
- Storing the same content twice returns the existing id with `deduplicated: true`. The dedupe check and the insert share one connection lock, so concurrent identical stores persist one active fact.
- graphify subprocesses are bounded: a build job is killed after 30 minutes, and `query_codebase` after 60 seconds. The helper drains stdout and stderr while it waits, so large output cannot deadlock the wait.
- The `brain_git_*` tools run `git` with the working directory set to the given path. Git does not climb past a mount boundary, so a non-repository path reports a hint to pass the repository root.
- `GIT_DISCOVERY_ACROSS_FILESYSTEM` is deliberately not set globally. Setting it would let git find an unrelated parent repository.

## Known limits

- Plain `cargo test` does not build the `extras` tests. Use `--all-features`, as the allowlist does.
- `verify --` prints its output only after the command finishes, so long runs show nothing until then.
- `doctor --fix` clears quarantine. Run it only on this repository's build output, not on a user's installed binary.
- Memory tests need a temporary `brain.db`. The real database is shared state.
- The stop gate only sees a repo that git reports. A change outside git is not checked.
- `graphify hook install` (`graphify/repos.rs`) runs with no timeout and inherited stdio, so it can wait on a prompt.
- `sessions::digest::tests::ingests_gemini_jsonl_digest` failed once in a parallel `--all-features` library run (left 3, right 1) and passes under `--test-threads=1`. It looks like parallel-test interference, and the cause is not investigated.

## Related

- Core routing logic: [core-crate](core-crate.md)
- Feature map: [[../features/cli-verify-and-memory]]
