# cli-verify-and-memory

Summary: agent-brain is the routing layer that every agent calls at turn start. Its CLI and MCP tools run a command and record its real exit, store and retrieve durable memory, install itself into hosts, and report its health, so that an agent's claims about what passed are backed by recorded exit codes.

## Sub-features

- `verify-record-exit`: Runs a command and records its real exit code as evidence.
  - Intent: Make "tests passed" a recorded fact instead of a claim.
  - Behaviour: `agent-brain verify [--source cli|orchestrator] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code and source.
  - Outcome: The exit code, source, and the summed `test result:` count (`tests_run`, schema v22, `src/verify.rs` `count_tests`) are recorded. A run without a cargo summary stores no count.
- `verify-init-scaffold`: Creates the harness files for a repository.
  - Behaviour: `agent-brain verify init` writes `.verify/verify.toml`, an executable `run.sh`, and `README.md`.
  - Outcome: The three files exist and `run.sh` is executable.
- `verify-report`: Aggregates recorded outcomes by source.
  - Behaviour: `agent-brain verify record` and `verify report`.
  - Outcome: A summary of recorded exits per source.
- `memory-put-and-flush`: Writes durable facts, with a retry queue.
  - Intent: Keep a fact even when the store is briefly unavailable.
  - Behaviour: `agent-brain memory put`, `enqueue`, and `flush-pending`.
  - Outcome: The fact is stored, or queued and later flushed.
- `route-task-context`: Returns ranked skills, rules, and memory for a turn.
  - Intent: Give the agent the context it needs before it acts.
  - Behaviour: `route_task` ranks candidates under a token budget, applies the core must-apply rules, and returns suggested tools.
  - Outcome: A structured route result. Core ranking is proven by the core crate (see `core-crate.md`).
- `install-and-doctor`: Installs MCP, hooks, and permissions, and checks health.
  - Behaviour: `agent-brain install --global`, `doctor`, `doctor --fix`, `index`, `briefing`.
  - Outcome: A health report. `doctor --fix` clears quarantine without re-signing.
- `sync-and-bootstrap`: Syncs the brain across machines.
  - Behaviour: `agent-brain sync git|cloud …` and `agent-brain bootstrap`.
  - Outcome: Memory is pushed or pulled from the sync repository.

## Code

- `agent-brain/src/mcp/server.rs`
  - Owns: `route-task-context` and the MCP tool surface, including the `grep_search` parameters (`query` alias, `path` default `"."`).
- `agent-brain/src/git_tools.rs`
  - Owns: the `brain_git_*` MCP tools. No other feature map covers them, so this feature owns them by assignment.

## How to get to it (user POV)

- CLI: `agent-brain verify`, `verify init`, `verify report`, `memory put`, `install --global`, `doctor`, `briefing`, `sync`
- MCP: `route_task`, `store_memory`, `verify_run`, `record_verify_outcome`, `file_summary`, `grep_search`, `read_file_head`, `read_file_tail`

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test -p agent-brain-core` (passes 11 of 11, see `.verify/artifacts/feature-map-baseline/core-unit.log`). The whole crate runs with `cargo test --all-features` (303 unit tests, 0 failed, recorded by `agent-brain verify`).
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
- Plain `cargo test` in this crate does not build the `extras` tests. Any command that bypasses the allowlist must pass `--features extras` or `--all-features`.
- `doctor --fix` clears quarantine. Run it only on this repository's build output, not on a user's installed binary.
- Memory tests must use a temporary database path. Writing to the real `brain.db` changes shared memory.
