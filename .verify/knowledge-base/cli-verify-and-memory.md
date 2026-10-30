---
feature: "[[../features/cli-verify-and-memory]]"
---

# CLI, verify, and memory

## What it does

Version 1.0.0-rc5 (`agent-brain/Cargo.toml`). See CHANGELOG.md for the release contents.

agent-brain is the routing layer every agent calls at the start of a turn. Its CLI and MCP tools run a command and record its real exit code as evidence. They also store and retrieve durable memory, install themselves into hosts, and report their own health.

## How it works

1. **Verify.** `agent-brain verify [--source cli|orchestrator] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code, source, the summed `test result:` count (`tests_run`, schema v22), and `code_hash` (schema v24), the hash of the code tree the run tested. A run with no cargo summary stores no count. With `--save-artifact` the captured output is also written to `${AUTONOMIC_HOME:-~/.autonomic}/artifacts/<project-folder>/<report-id>.log` (`src/verify.rs` `save_artifact_log`). The flag is opt-in, so the default run writes no log file.
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

- Extensions: the first one is `bench`, built from `extensions/bench` in this repo (`agent-brain-bench`, provides `bench` and `proofs`). Its manifest is in agent-registry PR #14 and its aarch64 build is a prerelease asset. `ext add bench` reads the registry's default branch, so it reports "not in the agent-registry cache" until PR #14 is merged. Test the install path against a seeded cache before that.
- The benchmark move: `eval --beam` is now `bench --beam`. `bench`, `proofs`, and the beam harness left the core binary. `persist_proof_snapshot` moved with `proofs`. The store exposes `bm25_fast_path_eligible` so the supervisor benchmark can read the BM25 fast-path decision.
- Extension trust: an install needs the manifest's SHA-256 to match and `codesign --verify --strict` to pass with TeamIdentifier `9HSMVVUMVX`. A failed check removes the download. Only macOS builds are accepted.
- `brain.db` is plain SQLite. The opt-in SQLCipher path (`AGENT_BRAIN_ENCRYPT_DB`, `src/db/at_rest.rs`) was removed, and the build no longer vendors OpenSSL. A database encrypted by an earlier build fails to open with an error that says to restore an unencrypted copy.
- `async-nats` was an unused dependency and was removed.
- Shell write: `agent-brain remember --text T --tag TAG [--tag TAG ...] [--source S]` stores one fact and prints `{id, deduplicated}`. The first tag becomes the fact's topic, and every tag is recorded in the additive `fact_tags` table (migration 25). The text is embedded the same way as an MCP write, and it is refused if it looks like a secret or is over 20,000 characters. There is no 50-word cap for shell writes. MCP `store_memory` keeps its cap (`src/remember.rs`).
- Corpus import: `agent-brain import-corpus DIR --tag TAG [--tag ...]` walks `*.md` files under DIR, splits paragraphs on blank lines, and stores each paragraph with source `<file path>#paragraph-N`. Duplicates are skipped by content hash, so importing the same folder twice stores nothing new. Secret and over-long paragraphs are counted and skipped. The report is JSON with `files`, `paragraphs`, `stored`, `deduplicated`, `rejected_secret`, and `rejected_too_long`.
- Recall matches a tag against the fact's topic or its `fact_tags` rows.
- Shell recall: `agent-brain recall [query words] [--tag T ...] [--limit N] [--json]` reads active facts directly from `brain.db` (`src/recall.rs`, `BrainStore::recall_rows`). Output is a JSON array of `{schema_version: 1, id, text, tags, created, source, score}`. `score` is null without a query. With a query, a fact must share at least one token with it, and results are sorted by the share of query tokens found. There are no embeddings and no model call. `--tag` matches a fact's `topic`, since facts carry one topic and no tag list. Empty results are `[]` with exit 0. Superseded and expired facts are excluded. Limit range is 1 to 200, default 20.
- Route stalls (root cause): when the daemon connection dropped, the stdio proxy (`serve_one_proxy_connection` in `src/mcp/server.rs`) aborted its watchdog and returned without answering the requests it had forwarded. The host waited until its own idle timeout (1800 s). The proxy also spawned a new daemon only once per process, so after a second daemon loss it never tried again. Fixed: pending requests are answered with a "connection was lost; retry" error (code -32000), and the spawn guard resets after each successful connect. Regression test: `proxy_answers_a_pending_request_when_the_daemon_drops_it` (fails without the fix, with a timeout). Takes effect once the binary is installed and the proxy session restarts.
- Daemon lifetime (`src/daemon_watchdog.rs`): `serve --daemon` checks every 30 seconds. It exits when its `serve.lock` is no longer the file it locked, when its socket is gone (for example, its home was deleted), or when no client session has been open for `AGENT_BRAIN_DAEMON_IDLE_SECS` (default 3600). A proxy starts a new daemon on its next connection. Before this, a daemon started under a temporary `HOME` outlived it, with its lock and database unlinked.
- Socket paths are limited to about 104 characters on macOS. Long homes make `serve` fail with "path must be shorter than SUN_LEN".
- Auto-capture is off by default, and it gates only automatic `auto-route` writes. Explicit `store_memory` writes are not affected (`capture_blocked` in `src/db/write_handler.rs`). Before this fix, turning auto-capture off also refused explicit writes with "auto capture disabled".
- Auto-capture is off by default. `route_task` used to store an `auto-route` fact on every turn that used MCP tools. Reads already hid those facts as index noise, so the default now skips the write. Set `AGENT_BRAIN_AUTO_CAPTURE=1` to turn it back on.
- `mcp add` warns about an empty header value and an unset `${VAR}` placeholder. A server with a bad key still lists its tools, so the warning is the only signal before a call fails.
- The MCP proxy answers any request that runs longer than 300 seconds with a timeout error (`PROXY_REQUEST_TIMEOUT` in `src/mcp/server.rs`). The upstream call keeps running after that, so the agent can miss a result that finishes later. The limit is one value for every request, not per upstream server. The proxy runs inside each host session's `serve` process, so a session started before the change keeps the old 30-second limit until its MCP server restarts.
- `mcp add --env` accepts both `--env KEY=VAL` and `--env=KEY=VAL`. Before the fix in `src/mcp_cli.rs` (`parse_env_flags`), the space-separated form was dropped silently, so the server ran with an empty env.
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
