# CLI, verify, and memory

## What it does

agent-brain is the routing layer every agent calls at the start of a turn. Its CLI and MCP tools run a command and record its real exit code as evidence. They also store and retrieve durable memory, install themselves into hosts, and report their own health.

## How it works

1. **Verify.** `agent-brain verify [--source cli|orchestrator] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code, source, and the summed `test result:` count (`tests_run`, schema v22). A run with no cargo summary stores no count.
2. **Harness scaffold.** `agent-brain verify init` writes `.verify/verify.toml`, an executable `run.sh`, and a README.
3. **Report.** `agent-brain verify record` and `verify report` summarize recorded exits per source.
4. **Memory.** `agent-brain memory put` writes a durable fact. `enqueue` and `flush-pending` hold facts that could not be written yet and retry them later.
5. **Route.** `route_task` ranks skills, rules, and memory for a turn under a token budget, applies the must-apply rules, and returns suggested tools. Ranking lives in the core crate (see [core-crate](core-crate.md)).
6. **Stop gate.** `agent-brain stop-gate` runs as the Stop hook. It blocks a stop when code changed in a repo and either the default verify step fails, or the feature map or knowledge base has not been updated (see below).
7. **Install and doctor.** `agent-brain install --global` writes MCP config, hooks, and permissions. `doctor` checks health, and `doctor --fix` clears quarantine.
8. **Sync.** `agent-brain sync git|cloud` and `bootstrap` move memory between machines.

### Stop gate rules

- A repo with changed files needs a passing `agent-brain verify` run recorded after the last edit. Only `cli` and `orchestrator` sources count.
- If the repo has `.verify/features/`, a code change also needs a changed file under `.verify/features/` and one under `.verify/knowledge-base/`. A passing verify run does not lift this block.
- A block is repeated up to three times for the same edit. After that the stop is allowed, and the gate prints why.

## Where the code is

- CLI and verify: `agent-brain/src/verify.rs`, `count_tests` for the cargo count
- Stop gate: `agent-brain/src/stop_gate.rs` (`changed_paths`, `feature_map_missing`, `feature_map_gap`, `evaluate_stop`)
- Host hooks: `agent-brain/src/host_hooks.rs` (`claude_code_gate_fragment` installs the `Stop` hook)
- MCP tools: `route_task`, `store_memory`, `verify_run`, `record_verify_outcome`, `file_summary`, `grep_search`, `read_file_head`, `read_file_tail`

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
- The `brain_git_*` tools run `git` with the working directory set to the given path. Git does not climb past a mount boundary, so a non-repository path reports a hint to pass the repository root.
- `GIT_DISCOVERY_ACROSS_FILESYSTEM` is deliberately not set globally. Setting it would let git find an unrelated parent repository.

## Known limits

- Plain `cargo test` does not build the `extras` tests. Use `--all-features`, as the allowlist does.
- `verify --` prints its output only after the command finishes, so long runs show nothing until then.
- `doctor --fix` clears quarantine. Run it only on this repository's build output, not on a user's installed binary.
- Memory tests need a temporary `brain.db`. The real database is shared state.
- The stop gate only sees a repo that git reports. A change outside git is not checked.

## Related

- Core routing logic: [core-crate](core-crate.md)
- Feature map: `agent-brain/.verify/features/cli-verify-and-memory.md`
