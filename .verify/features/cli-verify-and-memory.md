---
feature: cli-verify-and-memory
knowledge_base: "[[../knowledge-base/cli-verify-and-memory]]"
related:
  - "[[core-crate]]"
  - "[[packages]]"
  - "[[serve-daemon-and-proxy]]"
---

# cli-verify-and-memory

Summary: agent-brain is the routing layer that every agent calls at turn start. Its CLI and MCP tools run a command and record its real exit, store and retrieve durable memory, install itself into hosts, and report its health, so that an agent's claims about what passed are backed by recorded exit codes.

## Sub-features

- `verify-record-exit`: Runs a command and records its real exit code as evidence.
  - Intent: Make "tests passed" a recorded fact instead of a claim.
  - Behaviour: `agent-brain verify [--source cli|orchestrator] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code and source.
  - Outcome: The exit code, source, and the summed `test result:` count (`tests_run`, schema v22, `src/verify.rs` `count_tests`) are recorded. A run without a cargo summary stores no count. It also stores `code_hash`, the hash of the code tree the run tested, which the stop gate compares with the current tree.
- `verify-init-scaffold`: Creates the harness files for a repository.
  - Behaviour: `agent-brain verify init` writes `.verify/verify.toml`, an executable `run.sh`, and `README.md`.
  - Outcome: The three files exist and `run.sh` is executable.
- `verify-report`: Aggregates recorded outcomes by source.
  - Behaviour: `agent-brain verify record` and `verify report`.
  - Outcome: A summary of recorded exits per source.
- `memory-put-and-flush`: Writes durable facts, with a retry queue.
  - Intent: Keep a fact even when the store is briefly unavailable.
  - Behaviour: `agent-brain memory put`, `enqueue`, and `flush-pending`. The MCP `store_memory` and `delete_memory` tools send through the write queue on `spawn_blocking`, so they never park an async worker.
  - Outcome: The fact is stored, or queued and later flushed. A duplicate content hash returns the existing id with `deduplicated: true`, and the duplicate check and insert run under one connection lock, so concurrent identical stores persist one active fact.
- `route-task-context`: Returns ranked skills, rules, and memory for a turn.
  - Intent: Give the agent the context it needs before it acts.
  - Behaviour: `route_task` ranks candidates under a token budget, applies the core must-apply rules, and returns suggested tools.
  - Outcome: A structured route result. Core ranking is proven by the core crate (see [[core-crate]]).
- `install-and-doctor`: Installs MCP, hooks, and permissions, and checks health.
  - Behaviour: `agent-brain install --global`, `doctor`, `doctor --fix`, `index`, `briefing`.
  - Outcome: A health report. `doctor --fix` clears quarantine without re-signing.
- `sync-and-bootstrap`: Syncs the brain across machines.
  - Behaviour: `agent-brain sync git|cloud …` and `agent-brain bootstrap`.
  - Outcome: Memory is pushed or pulled from the sync repository.
- `feedback-report`: Opt-in telemetry report and agent feedback request, every `feedback_report.interval_hours` (default 3).
  - Intent: Show what the gates, hooks, and routing did, and collect the agent's own feedback on agent-brain and agent-spine.
  - Behaviour: With `feedback_report.enabled: true` (default false), the next `route_task` writes `reports/telemetry-<ts>.md` under the brain home and adds a `feedback-report` must-apply item asking the agent to write `reports/agent-feedback-<ts>.md`. The item repeats until that file exists. agent-spine receives it through its `route_task` forwarding, so it shares this flag.
  - Outcome: Telemetry for the window (hook events, sessions, gate denials, tool calls) and one agent feedback file per report. Settings are in `src/settings.rs`, the logic in `src/feedback_report.rs`.
- `feature-graph`: Prints the feature maps as a JSON graph.
  - Intent: Let an agent find the right feature map by graph search, and let Obsidian show the same links.
  - Behaviour: `agent-brain feature-graph [DIR]` reads each map's frontmatter `feature` and `related` wikilinks, and its `## Sub-features` and `## Code` sections. It rejects a map whose `feature` differs from its file name, a `related` entry that is not a `[[wikilink]]`, and a link to an unknown feature.
  - Outcome: JSON with `nodes` (id, sub_features, code) and `edges` (from, to). Logic is in `src/feature_graph.rs`.
- `hook-event-counter`: Records each host hook event so hook invocations per session can be counted.
  - Intent: Measure how often the route gate runs before changing its configuration.
  - Behaviour: `hooks/route_gate.py` appends `{timestamp, event, session}` to `hook_events.jsonl` in the brain home for every event that reaches `main()`. Events skipped by the disabled path are not counted.
  - Outcome: One JSON line per hook event, read by the feedback report. Logic is in `hooks/route_gate.py` and tests in `hooks/test_route_gate.py`.

## Code

- `agent-brain/src`
- `agent-brain/tests`
- `agent-brain/hooks`
- `agent-brain/Cargo.toml`
- `agent-brain/config.example.yaml`
- `Cargo.lock`

## How to get to it (user POV)

- CLI: `agent-brain verify`, `verify init`, `verify report`, `memory put`, `install --global`, `doctor`, `briefing`, `sync`
- MCP: `route_task`, `store_memory`, `verify_run`, `record_verify_outcome`, `file_summary`, `grep_search`, `read_file_head`, `read_file_tail`

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test -p agent-brain-core` (passes 11 of 11, see `.verify/artifacts/feature-map-baseline/core-unit.log`). The whole crate runs with `cargo test --all-features -- --skip graphify_1k_ci_gate --skip scale_1k_passes_gate --test-threads=1`, the allowlisted command.
- **Targeted tests for the memory and graphify fixes:** `cargo test -p agent-brain --all-features --lib -- store_and_delete_memory_round_trip concurrent_identical_stores_persist process::tests` (5 passed when run in this pass).
- **Stop-gate tests:** `cargo test -p agent-brain --lib stop_gate` covers the scope, the code-tree hash, the block limit, and the matrix of owned and unclaimed code.
- **Feature-graph and feedback-report tests:** `cargo test -p agent-brain --lib feature_graph`, `cargo test -p agent-brain --lib feedback_report`, and `cargo test -p agent-brain --bin agent-brain cli_help`.
- **Functional API Tests (Hurl):** not yet written for the CLI or MCP surface.
- **Allowlisted Verification:** `.verify/run.sh`, listed in `.verify/verify.toml`
- **Preconditions:** Rust toolchain. Memory tests need a temporary `brain.db` path, not the user's `~/.autonomic/memory`.

### Verification Steps

1. **core-unit-check** (sub-feature: route-task-context)
   - Request: `cargo test -p agent-brain-core`
   - Why: Proves the core ranking, decay, and contract rules.
   - Expected: 11 passed, 0 failed (verified in this pass).
   - Impact: A failure means routing logic changed. Fix the cause.
2. **core-boundary-check** (sub-feature: route-task-context)
   - Request: `sh .verify/checks/core_boundary.sh`
   - Why: Proves the core crate has no storage, runtime, or IO dependency.
   - Expected: Exit 0 with "core boundary holds" (verified in this pass).
   - Impact: A failure means an IO crate entered the core.

## Gotchas

- Resolved (2026-10-24): the adapter test targets `tests/v0_23.rs` and `tests/v0_30.rs` did not compile under plain `cargo test`. The cause was a feature gate, not missing code. The `graphify`, `grpc`, and `BrainStore` code-graph methods are behind `#[cfg(feature = "extras")]`, and `Cargo.toml` says CI and local proofs use `--features extras`. The allowlist ran plain `cargo test`, so the tests never compiled there. The allowlist now runs `cargo test --all-features`, which matches `.verify/playbooks/verify_full_suite.sh`. The recorded run passes 303 unit tests and every integration target, with 0 failed.
- Behaviour change (2026-10-24): `verify --` no longer streams the command's output live. Output is printed in full after the command ends, so long runs show nothing until they finish.
- The hook tests in `hooks/test_route_gate.py` run inside `.verify/playbooks/verify_multi_agent_enforcement.sh` with `python3 agent-brain/hooks/test_route_gate.py`, which `.verify/run.sh` calls. A passing `.verify/run.sh` therefore covers the Python hook.
- Plain `cargo test` in this crate does not build the `extras` tests. Any command that bypasses the allowlist must pass `--features extras` or `--all-features`.
- `doctor --fix` clears quarantine. Run it only on this repository's build output, not on a user's installed binary.
- Memory tests must use a temporary database path. Writing to the real `brain.db` changes shared memory.
- graphify subprocesses run through `graphify/process.rs` `output_with_timeout`. A graph build job is killed after 30 minutes and a `query_codebase` call after 60 seconds, so a hung graphify no longer holds `JOB_LOCK` or the MCP request forever. `graphify hook install` in `graphify/repos.rs` still runs with no timeout.
- Version: 1.0.0-rc5, set in `agent-brain/Cargo.toml`. `Cargo.lock` is the workspace lock and is listed in the Code section. See CHANGELOG.md for the release contents.
- Stop hook binary: the Stop hook runs `~/.local/bin/agent-brain`, which is installed separately from the release build. Until the rebuilt binary is installed, the gate runs the old scope rules (any change needs a run), and runs recorded by the old binary have no `code_hash`, so they do not count.
- Stop gate scope: only code paths need a passing run, and a run counts only for the code tree it tested (`code_hash`, from `src/verify.rs` `code_tree_hash`). Doc-only changes get the feature-map and knowledge-base owner check. The gate does not parse feature-graph frontmatter, because frontmatter is optional.
- Flaky under parallel tests: `sessions::digest::tests::ingests_gemini_jsonl_digest` failed once in a parallel `cargo test --all-features --lib` run (left 3, right 1). It passes under the allowlisted `--test-threads=1` run, which exits 0 (recorded by `agent-brain verify`). Cause not investigated.
