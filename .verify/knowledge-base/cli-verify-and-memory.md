---
feature: "[[../features/cli-verify-and-memory]]"
---

# CLI, verify, and memory

## What it does

Version 1.0.0-rc7 (`tekmor/Cargo.toml`). See CHANGELOG.md for the release contents.

tekmor is the routing layer every agent calls at the start of a turn. Its CLI and MCP tools run a command and record its real exit code as evidence. They also store and retrieve durable memory, install themselves into hosts, and report their own health.

## How it works

1. **Verify.** `tekmor verify [--source cli|orchestrator] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code, source, the summed `test result:` count (`tests_run`, schema v22), and `code_hash` (schema v24), the hash of the code tree the run tested. A run with no cargo summary stores no count. With `--save-artifact` the captured output is also written to `${AUTONOMIC_HOME:-~/.autonomic}/artifacts/<project-folder>/<report-id>.log` (`src/verify.rs` `save_artifact_log`). The flag is opt-in, so the default run writes no log file.
2. **Harness scaffold.** `tekmor verify init` writes `.verify/verify.toml`, an executable `run.sh`, and a README.
3. **Report.** `tekmor verify record` and `verify report` summarize recorded exits per source.
4. **Memory.** `tekmor memory put` writes a durable fact. `enqueue` and `flush-pending` hold facts that could not be written yet and retry them later. The MCP `store_memory` and `delete_memory` tools go through the write queue on `spawn_blocking`, so a slow write does not park an async worker.
5. **Route.** `route_task` ranks skills, rules, and memory for a turn under a token budget, applies the must-apply rules, and returns suggested tools. Ranking lives in the core crate (see [core-crate](core-crate.md)).
6. **Stop gate.** `tekmor stop-gate` runs as the Stop hook. It blocks a stop when code changed in a repo and either the default verify step fails, or the feature map or knowledge base has not been updated (see below).
7. **Install and doctor.** `tekmor install --global` writes MCP config, hooks, and permissions. `doctor` checks health, and `doctor --fix` clears quarantine.
8. **Sync.** `tekmor sync git|cloud` and `bootstrap` move memory between machines.
9. **Feedback report.** Opt-in with `feedback_report.enabled` (default false, interval `interval_hours`, default 3). When a report is due, `route_task` writes `reports/telemetry-<ts>.md` under the brain home and adds a `feedback-report` must-apply item telling the agent to write `reports/agent-feedback-<ts>.md` at the end of the current task, without asking the user for permission. The item covers what helped, what was wrong or noisy, likely false-positive blocks, missing capabilities, memory quality, and why listed skills were not applied, and it repeats until that file exists. `src/feedback_report.rs` holds the logic.
10. **Hook events.** `hooks/route_gate.py` appends each host hook event to `hooks/hook_events.jsonl` with its session ID. The feedback report reads this file. The hook script is compiled into the binary, so the file appears only after the hooks are reinstalled from a build of this source (`tekmor install --claude-code --global`). An older installed hook never writes it, and the report then shows zero sessions.
- **Bash gate builtins.** Shell keywords and builtins (`SHELL_BUILTINS` in `src/command_policy.rs`) are allowed without an "unrecognised command" warning. Cases are in `src/command_policy_cases.txt`. An unknown command warns that the user should allow it with `tekmor rules add`, since agents are blocked from changing those rules. `tekmor rules list|add|remove|reset` manage the rules (stored in `command-rules.db`, or the file named by `TEKMOR_COMMAND_RULES`).
11. **Feature graph.** `tekmor feature-graph [DIR]` prints the feature maps as JSON nodes and edges, built from frontmatter and the `## Sub-features` and `## Code` sections. `src/feature_graph.rs` holds the logic.

### Stop gate rules

- A passing run counts only for the code tree it tested. The gate hashes the changed code paths (under `src/`, `crates/`, `tests/`, or `hooks/`, or a `Cargo.*` file, never under `.verify/`) and compares that hash with the `code_hash` stored on each verify run. Editing docs does not invalidate a run.
- Only the repo the hook runs in is checked. A workspace folder that is not a repo is not scanned for child repos.
- The stop-gate unit tests run with `cargo test -p tekmor --lib stop_gate`, which is on the allowlist in `.verify/verify.toml` and can be run as a targeted check.
- A doc-only change runs no verify step. The feature-map and knowledge-base rules still apply to it.
- A repo with changed files needs a passing `tekmor verify` run recorded after the last edit. Only `cli` and `orchestrator` sources count.
- If the repo has `.verify/features/`, a code change also needs a changed file under `.verify/features/` and one under `.verify/knowledge-base/`. A passing verify run does not lift this block.
- A block is repeated up to three times for the same edit. After that the stop is allowed, and the gate prints why.

## Where the code is

- CLI and verify: `tekmor/src/verify.rs`, `count_tests` for the cargo count
- Stop gate: `tekmor/src/stop_gate.rs` (`changed_paths`, `feature_map_missing`, `feature_map_gap`, `evaluate_stop`)
- Host hooks: `tekmor/src/host_hooks.rs` (`claude_code_gate_fragment` installs the `Stop` hook; `install_gemini_hooks` installs the `AfterAgent` stop gate for Gemini and Antigravity)
- Gemini stop gate (2026-11-10): `install_gemini_hooks` writes an `AfterAgent` entry that runs `tekmor stop-gate` with a millisecond timeout (`gemini_stop_gate_entry`). Gemini CLI documents `AfterAgent` and accepts `decision: "block"`. Antigravity CLI replaced Gemini CLI on 2026-06-18. The installer's docs say it shares `~/.gemini/settings.json`, which is not verified. Its own hook file, `~/.gemini/config/hooks.json`, has a `Stop` event, owned by cmux, and was left alone. Not yet verified live on either CLI: a turn without a passing verify should get a retry prompt. The installed binary needs `tekmor install --gemini --global` before the hook is live.
- MCP tools: `route_task`, `store_memory`, `verify_run`, `record_verify_outcome`, `file_summary`, `grep_search`, `read_file_head`, `read_file_tail`
- Retrieval algorithm (route suggestions and memory search): `tekmor/docs/retrieval-algorithm.md`. Its numbers come from `db/store.rs` (`score_items_with_bm25`), `retrieval.rs`, `recall.rs`, `engine.rs` (per-type floors), and `memory_decay.rs`. Update it in the same change as any of those constants.
- Memory write path: `tekmor/src/mcp/server.rs` (`store_memory`, `delete_memory`), `tekmor/src/db/write_handler.rs` (`send_and_recv`), `tekmor/src/db/store.rs` (`store_fact_full`)
- graphify subprocess timeouts: `tekmor/src/graphify/process.rs` (`output_with_timeout`), used by `graphify/jobs.rs` and `graphify/query.rs`
- Session digest tests: `tekmor/src/sessions/digest.rs` tests that set `TEKMOR_SESSION_HOME` serialize on `SESSION_HOME_ENV`, because the variable is process-wide and the test harness runs tests in parallel.
- Tool and upstream log writers: `tekmor/src/observability.rs` (`log_native_tool_use` takes a `NativeToolUse` struct, `log_upstream_call` takes an `UpstreamCall` struct), called from `mcp/server.rs`

## How to use it

- `tekmor verify -- cargo test` to record a real result
- `tekmor install --global` to install hooks for every host
- `tekmor doctor` to check health

## How it is verified

- `cargo test --all-features` (about 300 unit tests plus every integration target)
- The stop gate tests in `stop_gate.rs`: the verification rules, the feature-map and knowledge-base rule, and the end-to-end cases with a real git repository
- `sh .verify/checks/core_boundary.sh` for the core crate's dependency boundary

## Behaviour that matters

- Hook tool events (2026-11-05): `ingest_hook_events_since` (`src/tool_events.rs`) no longer renames `hooks/tool_events.jsonl` after ingest. The rename hid the file from the feedback report, which read it directly and so showed 0 tool calls, and it dropped events outside a window for any later, wider window. Each row's id is the SHA-256 of its line and the insert is `INSERT OR IGNORE` (`src/db/store.rs` `insert_tool_log`), so repeated reads are idempotent. The ingest return value counts lines read in the window, duplicates included. Regression tests: `tool_events::tests`. `sessions::digest::tests::distills_session_trajectories_into_procedure_edges` fails in a full parallel `cargo test --lib` run and passes alone; it fails the same way on the code before this change, so it is a pre-existing, order-dependent failure, probably the shared `TEKMOR_SESSION_HOME` env var it sets.
- Stop block message (2026-11-05): `run_stop_gate` prefixes every block with `Repo <name> is blocked:` (`named_reason`, `src/stop_gate.rs`), so a block raised for agentrun is not read as an tekmor block. An earlier block reported `internal/container/args.go` as unclaimed. It came from agentrun's feature map before its `## Code` section listed `internal/container/`; the current map claims it and the gate allows that repo.
- Release 1.0.0-rc7 (2026-11-05): `rustls` is bumped to 0.23.46 for four Dependabot advisories (`Cargo.lock`). Its transitive `getrandom` moves to 0.4.2.
- Extensions: the first one is `bench`, built from `extensions/bench` in this repo (`tekmor-bench`, provides `bench` and `proofs`). Its manifest is in agent-registry PR #14 and its aarch64 build is a prerelease asset. `ext add bench` reads the registry's default branch, so it reports "not in the agent-registry cache" until PR #14 is merged. Test the install path against a seeded cache before that.
- The benchmark move: `eval --beam` is now `bench --beam`. `bench`, `proofs`, and the beam harness left the core binary. `persist_proof_snapshot` moved with `proofs`. The store exposes `bm25_fast_path_eligible` so the supervisor benchmark can read the BM25 fast-path decision.
- Extension trust: an install needs the manifest's SHA-256 to match and `codesign --verify --strict` to pass with TeamIdentifier `9HSMVVUMVX`. A failed check removes the download. Only macOS builds are accepted.
- `brain.db` is plain SQLite. The opt-in SQLCipher path (`TEKMOR_ENCRYPT_DB`, `src/db/at_rest.rs`) was removed, and the build no longer vendors OpenSSL. A database encrypted by an earlier build fails to open with an error that says to restore an unencrypted copy.
- `async-nats` was an unused dependency and was removed.
- Shell write: `tekmor remember --text T --tag TAG [--tag TAG ...] [--source S]` stores one fact and prints `{id, deduplicated}`. The first tag becomes the fact's topic, and every tag is recorded in the additive `fact_tags` table (migration 25). The text is embedded the same way as an MCP write, and it is refused if it looks like a secret or is over 20,000 characters. There is no 50-word cap for shell writes. MCP `store_memory` keeps its cap (`src/remember.rs`).
- Feedback fixes (2026-11-09): MCP proxy calls now have a deadline per request. Routing tools (`route_task`, `store_memory`, and the other non-long tools) get 60 seconds, and `verify_run`, `learn_from_url`, `trigger_deep_analysis`, `route_to_mcp`, `import_memory`, `export_memory` and `brain_gc` get 300 seconds (`src/mcp/server.rs`, `PendingRequest::new`). When a request has a `progressToken` in `_meta`, the proxy sends `notifications/progress` every 10 seconds. The timeout reply names the deadline that expired. The daemon side has no per-request deadline; a daemon that hangs inside a call is only caught by the proxy.
- Feedback fixes (2026-11-09): the Bash gate's `substitutions` (`src/commit_gate.rs`) skips single-quoted text, so a template literal inside `'...'` no longer blocks the command. Backticks and `$(...)` inside double quotes are still treated as live. Regression cases are in `src/command_policy_cases.txt`.
- Feedback fixes (2026-11-09): `doctor` reports a recorded MCP command that does not exist on disk, and `doctor --fix` realigns the entry to the stable path returned by `ensure_mcp_runnable_binary`, not the running binary's path. Homebrew Cellar paths are versioned, so an upgrade removes the one that was recorded.
- Feedback fixes (2026-11-09): the route gate's read denial (`hooks/route_gate.py`) leads with the scout step and the unlock window. The existing hook tests pass with it, but none asserts this wording, so a future change to the message would not be caught.
- Feedback fixes (2026-11-09): a route that leaves items out for budget adds a `budget` warning with the number of items. Phase inference keeps the last clear phase for the same working directory (`sticky_phase`, `src/engine.rs`). A repository with `.verify/run.sh` gets a `verify-precedence` must-apply rule. Regression test: `an_unclear_message_keeps_the_phase_of_the_last_clear_one`.
- Corpus import: `tekmor import-corpus DIR --tag TAG [--tag ...]` walks `*.md` files under DIR, splits paragraphs on blank lines, and stores each paragraph with source `<file path>#paragraph-N`. Duplicates are skipped by content hash, so importing the same folder twice stores nothing new. Secret and over-long paragraphs are counted and skipped. The report is JSON with `files`, `paragraphs`, `stored`, `deduplicated`, `rejected_secret`, and `rejected_too_long`.
- Recall matches a tag against the fact's topic or its `fact_tags` rows.
- Recall options: `--since 30d|12h|90m` keeps facts created within the window. `--semantic` (requires a query) embeds the query and scores facts by cosine similarity against their stored embeddings. A fact's embedding is the one written with it at store time, found in `indexed_items` by content hash. `--min-score F` (requires `--semantic`) keeps only facts at or above the score. A repeat check is `recall "<topic>" --tag posted --since 30d --semantic --min-score 0.8 --json`. The threshold is a starting point, not a tuned value.
- Tag convention (not enforced in code): `voice` for writing samples, `posted` for approved posts, `project` for project notes.
- `--help` is available for `recall`, `remember`, and `import-corpus`.
- Shell recall: `tekmor recall [query words] [--tag T ...] [--limit N] [--json]` reads active facts directly from `brain.db` (`src/recall.rs`, `BrainStore::recall_rows`). Output is a JSON array of `{schema_version: 1, id, text, tags, created, source, score}`. `score` is null without a query. With a query, a fact must share at least one token with it, and results are sorted by the share of query tokens found. There are no embeddings and no model call. `--tag` matches a fact's `topic`, since facts carry one topic and no tag list. Empty results are `[]` with exit 0. Superseded and expired facts are excluded. Limit range is 1 to 200, default 20.
- Route stalls (root cause): when the daemon connection dropped, the stdio proxy (`serve_one_proxy_connection` in `src/mcp/server.rs`) aborted its watchdog and returned without answering the requests it had forwarded. The host waited until its own idle timeout (1800 s). The proxy also spawned a new daemon only once per process, so after a second daemon loss it never tried again. Fixed: pending requests are answered with a "connection was lost; retry" error (code -32000), and the spawn guard resets after each successful connect. Regression test: `proxy_answers_a_pending_request_when_the_daemon_drops_it` (fails without the fix, with a timeout). Takes effect once the binary is installed and the proxy session restarts.
- Daemon lifetime (`src/daemon_watchdog.rs`): `serve --daemon` checks every 30 seconds. It exits when its `serve.lock` is no longer the file it locked, when its socket is gone (for example, its home was deleted), or when no client session has been open for `TEKMOR_DAEMON_IDLE_SECS` (default 3600). A proxy starts a new daemon on its next connection. Before this, a daemon started under a temporary `HOME` outlived it, with its lock and database unlinked.
- Recall eval (`src/eval.rs`, `src/fixture.rs`, `src/cli_extras.rs`): golden rows used to be embedded with the deterministic hash embedder while `eval --ci --live` queried with the real model, so the live result (3/7 memory) measured a mismatch, not retrieval. `eval --ci --live` also seeded those rows into the user's live `brain.db`. Fixed: seeding uses the engine's own embedder (`seed_eval_fixture_with`), and `--live` runs on an isolated temp DB with the ONNX model (`run_ci_eval_live`, `new_isolated_engine_with_model`). Measured after the fix: isolated memory 7/7, skills 13/14. Live memory 7/7, skills 14/14. The golden queries are close paraphrases of their facts, so they cannot separate retrieval changes. Harder cases and distractors are still to do. Memory writes are stricter now: `store_memory` (`src/db/write_handler.rs` `validate_memory_text`) requires a lowercase slug topic and a one-line fact of at least 10 words. The tool description (`src/mcp/server.rs`) asks agents to state what was decided, why, and where it applies, so the stored fact is useful on its own. `list_memory` returns the stored text unchanged, so the richness comes from what is written. Default build (2026-11-01): gc, graphify, host_hooks, packages, and stats are ungated again, as on master, so the default build that CI runs compiles. The earlier gating came from the base and broke it. Feedback report mitigations (2026-11-01): the telemetry hook section reads `hooks/hook_events.jsonl`, the session-tagged log that the route gate hook writes. It reported "none" because the installed hook predates that logging; reinstalling the hooks creates the file. Sessions show "unknown" when hook records carry no session id, instead of a misleading 0. The route request is injected once, when the report is created, and is worded as optional reference data, not a required step (`src/feedback_report.rs`). The stop gate's Code parser accepts a bullet with a note after the path (`src/feature_map.rs` `parse_code_section`). `tool_events::append_hook_event` has no caller. It is noted here and left in place. Session digests were written on every route because `Config::load` defaulted `session_ingest_enabled` and `session_digest_enabled` to on (`tekmor/src/config.rs`, `env_flag`). They now default to off, matching the settings defaults. Existing digest rows were deleted by id. Guardrails in the same validator (`INSTRUCTION_MARKERS`, `DUMP_MARKERS`) reject facts phrased as instructions to the agent, which would be replayed as context, and facts that are automatic digests. Route recall returns project facts stored with `scope: project` and the repo as `current_working_directory`.

Verification evidence: the `verification` skill in agstack asks that runs go through the allowlist and keep their logs under `.verify/artifacts/`. `tekmor verify` now writes each run's output to `.verify/artifacts/<id>.log` in any repo with a `.verify/` directory, and the stop gate counts a passing run only when that log exists. The stop hook does not invoke the skill itself; it checks this evidence. The observation pass is off by default (`src/settings.rs`), so it no longer writes `obs/*` rows. Route recall returns project facts stored with `scope: project` and the repo as `current_working_directory`; verified with `memorable-capture-gate`. The live store had 26 auto-generated rows (session digests, `obs/*`, `auto-route`, and dependency or shell-trace lines), and they were deleted by id. The installed binary does not include these source changes until it is rebuilt and reinstalled. The `dataset`, `digest`, `distill`, and `grpc` commands moved out of the core binary into the `tekmor-extras` extension (`extensions/extras`, see [[extras-extension]]). The `fixture` and `skills-sh` commands and the eval, fixture, and skills.sh modules moved into the bench extension (see [[bench-extension]]). The core library no longer contains `cli_extras`, `operator_digest`, `dataset`, `distill`, `spine_ingest`, `grpc`, `eval`, `fixture`, or `skills_sh`. The `graphify` command moved out of the core binary into the `tekmor-graphify` extension (`extensions/graphify`, see [[graphify-extension]]). Core keeps the graph read path for route code context (`tekmor/src/graphify/mod.rs`, `tekmor/src/db/graph_store.rs`). The `query_codebase`, `trigger_deep_analysis`, and `graphify_job_status` MCP tools stay in core, and call the extension with `graphify::rpc`. The `eval` command moved out of the core binary into the bench extension (`extensions/bench/src/cli.rs`), and core help points to it. See [[bench-extension]].
- Socket paths are limited to about 104 characters on macOS. Long homes make `serve` fail with "path must be shorter than SUN_LEN".
- Auto-capture is off by default, and it gates only automatic `auto-route` writes. Explicit `store_memory` writes are not affected (`capture_blocked` in `src/db/write_handler.rs`). Before this fix, turning auto-capture off also refused explicit writes with "auto capture disabled".
- Auto-capture is off by default. `route_task` used to store an `auto-route` fact on every turn that used MCP tools. Reads already hid those facts as index noise, so the default now skips the write. Set `TEKMOR_AUTO_CAPTURE=1` to turn it back on.
- `mcp add` warns about an empty header value and an unset `${VAR}` placeholder. A server with a bad key still lists its tools, so the warning is the only signal before a call fails.
- The MCP proxy answers any request that runs longer than 300 seconds with a timeout error (`PROXY_REQUEST_TIMEOUT` in `src/mcp/server.rs`). The upstream call keeps running after that, so the agent can miss a result that finishes later. The limit is one value for every request, not per upstream server. The proxy runs inside each host session's `serve` process, so a session started before the change keeps the old 30-second limit until its MCP server restarts.
- `mcp add --env` accepts both `--env KEY=VAL` and `--env=KEY=VAL`. Before the fix in `src/mcp_cli.rs` (`parse_env_flags`), the space-separated form was dropped silently, so the server ran with an empty env.
- Servers past `upstream_mcp.max_servers` (default 2, `src/settings.rs`) are dropped by `enabled_servers` with no error, so their tools never reach the index. A server that shows 0 tools in `mcp list` may simply be over the cap. Since the over-cap change (2026-11-11), `mcp list` prints `not indexed (over max_servers)` for those servers (`is_over_max_servers` in `src/upstream/mod.rs`). Raise `max_servers` in `config.yaml` to index them. `mcp sync` does not name them.
- `grep_search` accepts `query` as an alias for `pattern`, and `path` defaults to `"."`. Callers that send `query` no longer fail with a missing-field error.
- Storing the same content twice returns the existing id with `deduplicated: true`. The dedupe check and the insert share one connection lock, so concurrent identical stores persist one active fact.
- graphify subprocesses are bounded: a build job is killed after 30 minutes, and `query_codebase` after 60 seconds. The helper drains stdout and stderr while it waits, so large output cannot deadlock the wait.
- The `brain_git_*` tools run `git` with the working directory set to the given path. Git does not climb past a mount boundary, so a non-repository path reports a hint to pass the repository root.
- `GIT_DISCOVERY_ACROSS_FILESYSTEM` is deliberately not set globally. Setting it would let git find an unrelated parent repository.
- The bash gate classifies the command after a shell keyword (`then`, `do`, `{`, `!`, and the rest of `COMMAND_PREFIX_KEYWORDS` in `src/command_policy.rs`). Before 2026-11-13 a segment that began with one of these was allowed whole, so `if true; then rm -rf /; fi` passed.
- Heredoc bodies are data unless the heredoc's own line names a shell or interpreter (`HEREDOC_EXECUTORS`, `INTERPRETERS`). A heredoc is recognised only when `<<` is outside quotes and arithmetic and its closing delimiter line exists; otherwise every line is classified as before. `$(...)` is classified in all heredocs, quoted or not.
- Hook-observed shell commands are stored in `verify_outcomes` with source `hook`, but only `cli` and `orchestrator` rows count as verification (`ran_a_check` in `src/verify.rs`). Older rows are unchanged; they simply stop appearing as recipes or as the last verification.
- Gate events recorded before 2026-11-13 include entries written by the test suite, about 90 copies of each case in `tests/bash_gate.rs`. Counts from `doctor` and the feedback report over that period overstate real blocks.

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

- Formatting pass (rustfmt, 2026-11-07): line breaks and spacing only, no behaviour change.
- Receipts need commit tracking: `code_tree_hash` covers only uncommitted code paths, so on a clean tree it is the same for every commit. The receipt therefore also requires `head_sha` (git HEAD, schema v26). A receipt only matches a run recorded after v26; older runs show NOT VERIFIED until re-run.
- Egress guard: network use is found by markers (`ureq::`, `opendal::`, `Command::new("curl")`, git `fetch`/`clone`/`pull`/`push` argument lists). A new client that uses a different API will not be caught. Add its marker to `NETWORK_CLIENT_MARKERS` when you add one.
- Dashboard savings: the bounded-read saving sums the bytes of every file in which a directory `grep_search` finds a match (`full_bytes` in `src/token_tools.rs`), whether or not an agent would have opened it. Read it as an upper bound, never as a measured saving. The dashboard labels it that way.
- Route stats recorded before 2026-11-13 mixed upstream MCP calls into route latency and cache-hit rate. The slow "routes" in that period (72 of 413 over five seconds on one machine) were `upstream_call` rows; real routes ran in 100 to 500 ms.
- `still-applies` is a reserved `must_apply` topic written by the MCP handler, not by a rule. A client that needs every constraint in full on each call should read them on the first route of the session, or after the 30-minute resend.
- `context_bundle.team_rules[].text` is empty by design since 2026-11-13. Read rule text from `applicable_rules`.

## Stop gate hand-back, watch mode, and the verify-init guard

- The stop gate no longer ends a turn silently after the block limit. `StopOutcome::HandBack` produces a `systemMessage` with NOT VERIFIED, and `hook_output` never sets a `decision` key for it, so the host still lets the turn end. Hosts that do not show `systemMessage` will not show the line; check each host (`src/host_hooks.rs`, `gemini_stop_gate_entry`) before relying on it.
- Watch mode (`gates.stop: watch` or `TEKMOR_STOP_GATE=watch`) never writes block state, so the block counter is not advanced and nothing is ever blocked. The reason is still computed, so the would-be block is recorded as `stop-would-block`.
- `tekmor verify init --force` is no longer blocked by the Bash guard. Plain `verify init` writes only when the file is missing.

## Protected checks, requirements, and route reuse

- A requirement line starts with `R<number>.` at the start of a line. Its check is the first backticked command after `check:`, on the same line or on the next indented line. An `R<n>` inside a list item or heading is not a requirement.
- Edits to `.verify/verify.toml`, `.verify/run.sh` and `.verify/checks/` are allowed. The receipt shows when they differ from the last commit.
- `init_verify` allowlists only the detected test commands and `.verify/run.sh`. A project without a recognised build file must add its test command by hand.

## Horizon 2 notes

- Rules are indexed only from `.mdc`, entry files and `rules/` directories. A stored rule from before the change is removed on the next sync.

- The lesson excerpt is the first failure-marker line of the failing run's saved output. A run with no saved output creates no lesson.

- `lessons` and `file_context` are filled from the route's `open_files`; a route with no open files returns neither.
- Lesson paths are the paths changed at the failing run that are still changed at the passing run. Each verify row stores its changed code paths (`changed_paths_json`, migration v28), and `learn_from_fix` in `src/lessons.rs` intersects them.
- `init_verify` adds `bundle exec rspec` only when the Gemfile names rspec. A Gemfile without rspec yields no Ruby test command.
- Context feedback credits a routed item only when the agent read its source path in that route (`opened_paths_for_route`, `src/db/store.rs`). Route items store `source_path` for this join.
- The index pruning test covers rules dropped on sync (prune_removes_stored_rules_that_are_not_rule_files, src/db/store.rs).
- Index and package coverage: index.rs tests cover parsing, classification and skip rules; tests/index_sync.rs drives sync_index_opts against a temp HOME. packages/mod.rs tests cover registry, removal, source parsing and git clone/update against local repositories.

## H1 and H2 completion

- Every H1 and H2 item in the delegation plan now has code and a test, except H2-6 (retrieval eval). That item needs owner approval for the new cases and for adding them to the gate.
- Lessons past their TTL are deleted by `run_gc` (`lessons_removed`), not only hidden on read.
- The eval gate is not part of `.verify/run.sh`; see bench-extension for its current 15 of 28 result.

## Estimated savings removed (R-9)

- The route briefing no longer prints an estimated savings figure. It shows the index size and routed tokens against the budget.
- The retrieval log no longer writes `saved_pct`. The stored column stays, and new rows write it as empty.
- Stats no longer report `total_saved_tokens`, `avg_saved_pct` or `routes_with_savings`. The dashboard headline counts tool savings only.
- The estimate assumed every indexed item would be loaded, so it was never a measurement.

## Repo-level host files

- The root `AGENTS.md` and the `.cursor/` folder were removed from the repository. They were project-install output, not runtime inputs. `tekmor install` or `tekmor mode install` recreates them in a project that needs them.

## Verified recipes need execution intent (R-4)

- `verified-recipe` is no longer a must-apply entry. Recipes come back in `verified_recipes` only when the request asks to run something (test, build, check, verify and similar).

## Retrieval stats deduped by topic (R-17)

- A route lists each memory topic once in `retrieval_stats`, even when two memories share it.

## Retrieval stats dedupe is tested (R-17)

- The dedupe is the pure function `memory_retrieval_stats` in `tekmor/src/engine.rs`, with a regression test.

## Context bundle dropped on core surface (R-8)

- Under `mcp.tool_surface: core`, `route_task` over MCP omits `context_bundle`. It duplicates top-level fields. The gRPC response keeps it, and the field can go after one release with no consumer.

## Savings labels (R-10)

- Bounded-read savings are an upper bound: the full size of each matched file, minus tokens returned. The dashboard labels them that way. The old "naive load" tile wording is gone (R-9 removed its estimate).

## Upstream tool suggestions need a name match (R-5)

- An upstream tool is suggested only when the query matches its server or tool name. A match on the description alone scores zero.

## Static route text sent once per session (R-6, R-16)

- MCP route responses send static warnings and the native-tool list once per session.

## Hook rows are not verify outcomes (R-11)

- The host hook no longer records shell commands as verify outcomes. 643 of 884 old rows were `cd`, `grep`, `git` and `ls`, so the table was mostly noise.
- Readers: the stop gate, receipt and recipes already counted only `cli` and `orchestrator` rows. The upstream historical adjustment in `tekmor/src/upstream/suggest.rs` read every source, so it loses the hook history as those rows age out.

## Dollar estimate removed (v29)

- Turn telemetry no longer stores or reports `estimated_dollars_saved`. Schema v29 drops the column.
- The figure was cache-read tokens times list price, with no measured baseline, so it was an estimate, not a saving.

## Route gate tool classification in Rust (R-15 step 1)

- `classify_tool` mirrors the Python route gate name rules. It is pure and unused until the route-gate subcommand lands (ADR 0001).

## Route gate decision in Rust (R-15 step 2, ADR 0001)

- `decide` is pure: it takes tool name, route state and time. It is not called yet. Subcommand, state I/O and shadow mode come in step 3.

## Route gate shadow (R-15 step 3, ADR 0001 amendment 2)

- Set TEKMOR_ROUTE_GATE_SHADOW=1 to log Python vs Rust route-gate disagreements to hooks/route_gate_shadow.jsonl. Off by default: the shadow call breaks the hook latency budget.
