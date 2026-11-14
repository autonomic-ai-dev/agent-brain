---
feature: cli-verify-and-memory
knowledge_base: "[[../knowledge-base/cli-verify-and-memory]]"
related:
  - "[[core-crate]]"
  - "[[packages]]"
  - "[[serve-daemon-and-proxy]]"
---

# cli-verify-and-memory

Summary: tekmor is the routing layer that every agent calls at turn start. Its CLI and MCP tools run a command and record its real exit, store and retrieve durable memory, install itself into hosts, and report its health, so that an agent's claims about what passed are backed by recorded exit codes.

## Sub-features

- `verify-record-exit`: Runs a command and records its real exit code as evidence.
  - Intent: Make "tests passed" a recorded fact instead of a claim.
  - Behaviour: `tekmor verify [--source cli|orchestrator] [--save-artifact] -- <cmd>` runs the command, captures its output, prints it when the command finishes, and stores the exit code and source. With `--save-artifact` the captured output is also written to `${AUTONOMIC_HOME:-~/.autonomic}/artifacts/<project-folder>/<report-id>.log`, and the path is printed to stderr.
  - Repo artifacts: when the repo has a `.verify/` directory, every run also writes its captured output to `.verify/artifacts/<report-id>.log`. The stop gate counts a passing run only when that log exists (`src/stop_gate.rs` `load_evidence`, `src/verify.rs` `repo_artifact_path`). Runs recorded before this rule have no log and no longer count.
  - Outcome: The exit code, source, and the summed `test result:` count (`tests_run`, schema v22, `src/verify.rs` `count_tests`) are recorded. A run without a cargo summary stores no count. It also stores `code_hash`, the hash of the code tree the run tested, which the stop gate compares with the current tree. A saved log lives under the artifacts folder; without the flag no log is written, and `.verify/artifacts/` is only written by project scripts.
- `extensions` (bench, graphify, and extras are the first three; see [[graphify-extension]] and [[extras-extension]]): Installs optional `tekmor-<name>` executables from agent-registry and runs them as subcommands.
  - Intent: Keep the core binary small by moving optional features out of it.
  - Behaviour: `tekmor ext list`, `ext add <name>`, `ext remove <name>`. `ext add` reads `extensions/<name>/extension.json` from the agent-registry checkout in the registry cache, downloads the build for this platform, checks its SHA-256 against the manifest, and installs it to `<brain home>/extensions/bin/tekmor-<name>`. An unknown subcommand that matches an installed extension runs it with the remaining arguments, with `TEKMOR_HOME` and `TEKMOR_SOCKET` (the serve socket) set. Its exit code is returned.
  - Outcome: The first extension is `bench` (`extensions/bench`), which provides `bench` (including `--beam`) and `proofs`. Extensions reach the brain through the serve socket as MCP clients, not by opening `brain.db`. `src/extensions.rs` holds the logic; tests cover name validation, SHA-256 checks, manifest loading, listing, and removal.
- `upstream-mcp-add`: Registers an upstream MCP server that tekmor proxies through `route_to_mcp`.
  - Behaviour: `tekmor mcp add <name> [--env KEY=VAL | --env=KEY=VAL] -- <command> [args...]` saves the server in `config.yaml` under `upstream_mcp`, then indexes its tools.
  - Outcome: The server and its env are stored, and its tool count is printed. `src/mcp_cli.rs` `parse_env_flags` reads both `--env` forms. `config_warnings` prints a `warning:` line for an empty header value or a `${VAR}` placeholder that is set neither in the server env nor in the environment. Indexing only proves the tools are listed, not that calls will authenticate.
- `upstream-mcp-over-cap`: An enabled upstream server past `upstream_mcp.max_servers` is not indexed, and `mcp list` says so.
  - Behaviour: `enabled_servers` keeps the first `max_servers` enabled servers. `is_over_max_servers` (`src/upstream/mod.rs`) flags the rest. `mcp list` shows `-` in the TOOLS column and `not indexed (over max_servers)` for each flagged server (`src/mcp_cli.rs`).
  - Outcome: Unit tests `upstream::tests::enabled_server_past_max_servers_is_over_cap` and `upstream::tests::disabled_servers_and_disabled_upstream_are_not_over_cap`. `mcp sync` still prints only a count and does not name skipped servers.
- `verify-init-scaffold`: Creates the harness files for a repository.
  - Behaviour: `tekmor verify init` writes `.verify/verify.toml`, an executable `run.sh`, and `README.md`.
  - Outcome: The three files exist and `run.sh` is executable.
- `verify-report`: Aggregates recorded outcomes by source.
  - Behaviour: `tekmor verify record` and `verify report`.
  - Outcome: A summary of recorded exits per source.
- `memory-put-and-flush`: Writes durable facts, with a retry queue.
  - Intent: Keep a fact even when the store is briefly unavailable.
  - Behaviour: `tekmor memory put`, `enqueue`, and `flush-pending`. The MCP `store_memory` and `delete_memory` tools send through the write queue on `spawn_blocking`, so they never park an async worker.
  - Feedback report: the telemetry hook section reads `hooks/hook_events.jsonl`, the session-tagged log the route gate hook writes; sessions show "unknown" when hook records carry no session id. The route request is injected once, as optional data. The stop gate's Code parser accepts a note after a path (`src/feature_map.rs`).
  - Default build: the core modules `gc`, `graphify`, `host_hooks`, `packages`, and `stats` are ungated again, matching master, so `cargo build -p tekmor` without features compiles (CI build stage). `lib.rs` keeps only the extras gates that still apply.
  - Session digests: `session_ingest_enabled` and `session_digest_enabled` default to off in `Config::load` (`src/config.rs`). Before this, they defaulted on, so every route wrote `session-digest-*` rows. Set `TEKMOR_SESSION_INGEST=1` to opt in.
  - Validation: `store_memory` takes a lowercase slug topic of at most 60 characters and a one-line fact of at least 10 words (and at most 50). The tool description asks for what was decided, why, and where it applies. Guardrails reject a fact that reads as an instruction to the agent (for example "ignore previous instructions" or "system:") and a fact that is an automatic digest or transcript line (for example "session digest" or "shell trace shows"), because stored facts are returned to later agents as context. Observation runs are off by default (`observation.enabled: false`), so stored facts no longer produce `obs/*` rows.
  - Outcome: The fact is stored, or queued and later flushed. A duplicate content hash returns the existing id with `deduplicated: true`, and the duplicate check and insert run under one connection lock, so concurrent identical stores persist one active fact.
- `route-task-context`: Returns ranked skills, rules, and memory for a turn.
  - Intent: Give the agent the context it needs before it acts.
  - Behaviour: `route_task` ranks candidates under a token budget, applies the core must-apply rules, and returns suggested tools. It stores an `auto-route` memory for the previous turn only when `TEKMOR_AUTO_CAPTURE=1` is set (off by default, `src/config.rs`).
  - Outcome: A structured route result. Core ranking is proven by the core crate (see [[core-crate]]).
- `memory-recall-eval`: Measures whether the right memory and skill come back for a query.
  - Intent: Give retrieval changes a number to move, instead of a claim that it got better.
  - Behaviour: `tekmor eval --ci`, now provided by the bench extension (see [[bench-extension]]), seeds golden memory and skill rows into an isolated temp DB with the deterministic embedder and reports Recall@3. `tekmor eval --ci --live` does the same with the real embedding model. Golden rows are embedded with the same embedder the route query uses, and neither mode writes to the user's `brain.db`.
  - Outcome: A JSON report with memory and skill Recall@3 and the failing queries. The CI gate fails below 0.85 for either suite. The deterministic run only exercises lexical matching, so a pass there does not show semantic accuracy.
- `install-and-doctor`: Installs MCP, hooks, and permissions, and checks health.
  - Behaviour: `tekmor install --global`, `doctor`, `doctor --fix`, `index`, `briefing`.
  - Outcome: A health report. `doctor --fix` clears quarantine without re-signing.
- `sync-and-bootstrap`: Syncs the brain across machines.
  - Behaviour: `tekmor sync git|cloud …` and `tekmor bootstrap`.
  - Outcome: Memory is pushed or pulled from the sync repository.
- `feedback-report`: Opt-in telemetry report and agent feedback request, every `feedback_report.interval_hours` (default 3).
  - Intent: Show what the gates, hooks, and routing did, and collect the agent's own feedback on tekmor and thesmos.
  - Behaviour: With `feedback_report.enabled: true` (default false), the next `route_task` writes `reports/telemetry-<ts>.md` under the brain home and adds a `feedback-report` must-apply item telling the agent to write `reports/agent-feedback-<ts>.md` at the end of the current task, without asking the user for permission. The item repeats until that file exists. thesmos receives it through its `route_task` forwarding, so it shares this flag.
  - Outcome: Telemetry for the window (hook events, sessions, gate denials, tool calls) and one agent feedback file per report. Settings are in `src/settings.rs`, the logic in `src/feedback_report.rs`.
- `feature-graph`: Prints the feature maps as a JSON graph.
  - Intent: Let an agent find the right feature map by graph search, and let Obsidian show the same links.
  - Behaviour: `tekmor feature-graph [DIR]` reads each map's frontmatter `feature` and `related` wikilinks, and its `## Sub-features` and `## Code` sections. It rejects a map whose `feature` differs from its file name, a `related` entry that is not a `[[wikilink]]`, and a link to an unknown feature.
  - Outcome: JSON with `nodes` (id, sub_features, code) and `edges` (from, to). Logic is in `src/feature_graph.rs`.
- `hook-event-counter`: Records each host hook event so hook invocations per session can be counted.
  - Intent: Measure how often the route gate runs before changing its configuration.
  - Behaviour: `hooks/route_gate.py` appends `{timestamp, event, session}` to `hook_events.jsonl` in the brain home for every event that reaches `main()`. Events skipped by the disabled path are not counted.
  - Outcome: One JSON line per hook event, read by the feedback report. Logic is in `hooks/route_gate.py` and tests in `hooks/test_route_gate.py`.
  - Bash gate: shell keywords and builtins (`for`, `export`, `set`, `echo`, `true`, and similar) are allowed without a warning (`SHELL_BUILTINS` in `src/command_policy.rs`, cases in `src/command_policy_cases.txt`). Commands with no default rule still warn. The warning asks the user to allow the command with `tekmor rules add`, because the bash gate blocks agents from changing command rules.
- `hook-event-ingest`: Merges hook tool events into the stats database.
  - Intent: Count tool calls in `stats` and the route window without consuming the file that the feedback report reads.
  - Behaviour: `ingest_hook_events_since` (`src/tool_events.rs`) reads `hooks/tool_events.jsonl`, inserts each record at or after the window start into `tool_log`, and leaves the file in place. Each row's id is the SHA-256 of its JSON line and inserts use `INSERT OR IGNORE`, so re-reading the file or overlapping windows never double count.
  - Outcome: Tool calls per window are counted once. Records outside a window stay in the file for a wider window. Regression tests: `tool_events::tests` (the log survives ingest, repeated ingest does not double count, a narrow window keeps events for a wider one).
- `verify-receipt`: `tekmor verify receipt` shows whether the current commit and code state passed its verify run.
  - Intent: A receipt a person can paste into a pull request, and that cannot pass on stale evidence.
  - Behaviour: `receipt_for_project` (`src/receipt.rs`) reads the newest `verify_outcomes` row for the project whose `head_sha` (git HEAD) and `code_hash` (uncommitted code paths) both match now, and whose source is `cli` or `orchestrator`. Exit 0 only when that run exited 0. `--markdown` and `--json` render the same verdict.
  - Outcome: Verdicts `PASSED`, `FAILED`, `NOT VERIFIED`. Rows recorded before schema v26 have no `head_sha` and never count. Regression tests: `receipt::tests` (13 cases) and `tests/first_receipt.rs` (four end-to-end runs covering a fresh home, stale code, a new commit, and markdown/json).
- `receipt-artifact`: A passing or failing receipt names the saved run output.
  - Behaviour: `receipt_for_project` points at `.verify/artifacts/<outcome id>.log` when that file exists. `verify --` saves it in any project with a `.verify/` folder.
  - Outcome: Text shows `artifact:` and markdown shows `Proof artifact`. Regression tests: two `receipt::tests` cases and an existence check in `tests/verify_layout.rs`.
- `project-mode-blocks`: A project install writes the agstack mode into the file each host reads.
  - Behaviour: `upsert_managed_block` (`src/host_install.rs`) writes text between `<!-- tekmor:begin <id> -->` and `<!-- tekmor:end <id> -->`. It leaves other content alone and does nothing on a second run. The VS Code copilot file uses a `copilot` block, so a team's own instructions are kept.
  - Outcome: Regression tests: `tests/project_mode.rs` (five hosts, idempotent, team content kept).
- `agstack-skill-usage`: The stats count routes whose top results included the agstack-mode or verification skill.
  - Behaviour: `agstack_usage` (`src/stats.rs`) counts `skill` items named `agstack-mode` or `verification` in each route log row. It shows in `stats`, `stats --markdown`, and a dashboard tile. It shows what route_task offered, not whether the agent read the skill.
  - Outcome: Regression tests: `tests/agstack_usage.rs` (unit cases and a real route inserted and counted through the CLI).
- `verify-init-layout`: `tekmor verify init` writes the agstack verification layout.
  - Intent: the harness an agent sets up matches the agstack verification skill, so its runs produce the same evidence.
  - Behaviour: `run.sh` runs phases in order (doctor, unit, static, functional), skips a phase the repository lacks with a stated reason, takes an optional feature argument, and writes `summary.txt` and one log per phase under `.verify/artifacts/<RUN_ID>/`. A failing doctor stops the run.
  - Outcome: Regression tests: `tests/verify_layout.rs` (five cases).
- `procedure-reuse`: A procedure learned from verify runs is recalled for a second task.
  - Intent: a repeatable demo of the claim that verified runs make later work faster.
  - Behaviour: two `verify --` runs for the same command produce trajectories that `retrieval::recall_procedures` groups into one procedure with total_runs 2 and success_rate 0.5.
  - Outcome: Regression tests: `tests/procedure_reuse.rs`. Demo: `docs/procedure-reuse-demo.md`.
- `dashboard-tiles`: Each dashboard tile opens to show what it counts and its source.
  - Behaviour: each card is a `<details>` element, so it works without scripts. The detail text states whether a figure is measured or an estimate.
  - Outcome: Regression test: `tests/dashboard_tiles.rs`.
- `egress-statement`: Every network path tekmor can open is listed in `docs/egress.md`.
  - Intent: A reviewer or user can check the no-telemetry and no-LLM claims against the code.
  - Behaviour: `tests/egress_guard.rs` finds files that use a network client or spawn `curl` or git network subcommands, and fails unless they match `EXPECTED_NETWORK_FILES`. It also checks that no LLM provider host appears in source and that no telemetry SDK is a dependency.
  - Outcome: Adding a network path requires updating the doc and the list together. Regression tests: the three `egress_guard` tests.
- `dashboard-measured-figures`: The local dashboard labels each figure by how it is produced.
  - Intent: Savings shown to a user must be measured or clearly marked as an estimate or upper bound.
  - Behaviour: `tekmor dashboard` renders `OperatorStats` (`src/stats.rs`). Its daily series comes from `BrainStore::daily_usage_since` (`src/db/store.rs`): routed tokens from `retrieval_log`, bounded-read saving from `tool_log.tokens_saved`, and verify passed/failed from `verify_outcomes.exit_code`. The bounded-read saving is an upper bound (bytes scanned ÷ 4 minus tokens returned). The naive-load figure assumes 120 tokens per indexed item.
  - Breakdowns: daily route calls, cache hit rate, and context tokens; latency buckets from `retrieval_log`; route phases; bounded tools by calls from `tool_log`; verify runs by command from `verify_outcomes`; indexed items by type. Every label from data is HTML-escaped.
  - Outcome: Charts are inline SVG and bars are HTML, with no network calls. Regression tests: `dashboard::tests` (empty period, scaling, zero values, escaping, stacked rows, line clamping) and `db::store::tests::dashboard_breakdowns_count_only_recorded_rows`.
- `stop-gate-repo-name`: The stop-gate block names the repo it blocks.
  - Intent: A block raised while working in one repo must not look like it belongs to another.
  - Behaviour: `run_stop_gate` prefixes each block reason with `Repo <name> is blocked:` (`named_reason` in `src/stop_gate.rs`). The gate evaluates only the git root of the session's cwd.
  - Outcome: The block headline names the repo. Regression test: `stop_gate::tests::blocking_reason_names_the_repo`.
- `gemini-stop-gate`: Gemini-family hosts run the stop gate at the end of each turn.
  - Behaviour: `install_gemini_hooks` (`src/host_hooks.rs`) registers `AfterAgent` in the Gemini settings file and runs `tekmor stop-gate`, with a timeout in milliseconds (`gemini_stop_gate_entry`). Reinstalling replaces only tekmor's entry and keeps user hooks.
  - Outcome: Regression tests: `host_hooks::tests::gemini_stop_gate_timeout_is_in_milliseconds` and `host_hooks::tests::gemini_stop_gate_reinstall_replaces_own_entry_and_keeps_user_hook`. Not yet verified live: whether the Gemini CLI and Antigravity CLI run `AfterAgent` and honor its block decision.
- `retrieval-algorithm-doc`: The scoring used for route suggestions and memory search is written down.
  - Behaviour: `tekmor/docs/retrieval-algorithm.md` gives the candidate sources (BM25, HNSW, RRF), the base blend, every adjustment, the thresholds, and the keyword and semantic memory modes, each with its source location.
  - Outcome: Documentation only. Its constants are checked by hand against the code, not by a test.

- `route-budget-report`: A route that drops items to stay within its token budget says so.
  - Behaviour: `try_add_route_item` (`src/engine.rs`) counts each item it leaves out, and the route appends a `budget` warning to `warnings` that names the count and the budget. The warning points to `skill_search` and `explain_last_context`.
  - Outcome: Callers can see that context was omitted, instead of receiving a shorter list with no reason.
- `phase-stickiness`: A message with no phase keyword keeps the last clear phase for the same directory.
  - Behaviour: `sticky_phase` (`src/engine.rs`) applies only when no phase is given explicitly. An inferred phase other than `unknown` replaces the stored one.
  - Outcome: Route labels no longer flip between `planning`, `implementing` and `unknown` on a session's messages.
- `verify-precedence-rule`: A repository with its own harness is told that its harness is the required check.
  - Behaviour: When `.verify/run.sh` exists under the working directory, the route adds a `verify-precedence` must-apply rule (`src/engine.rs`). It asks for the harness to run and for the run to be recorded with `tekmor verify`.
  - Outcome: The verified-recipe rule and the repository policy no longer point at different commands without saying which one wins.
- `bash-gate-quoted-text`: Quoted text is not treated as a live command substitution.
  - Behaviour: `substitutions` (`src/commit_gate.rs`) skips text inside single quotes, and still treats backticks and `$(...)` inside double quotes as live. The rule is covered by `src/command_policy_cases.txt`.
  - Outcome: A Node script that contains a template literal inside single quotes is no longer blocked as "computed at runtime".
- `doctor-mcp-missing-command`: Doctor says when the recorded MCP command is missing on disk.
  - Behaviour: `src/doctor.rs` reports "command does not exist on disk" for a mismatch whose recorded path is gone. `doctor --fix` realigns the entry to the stable path from `ensure_mcp_runnable_binary`, not to the running binary.
  - Outcome: A removed Homebrew Cellar path produces a specific message, and the fix does not record another versioned path.
- `read-denial-message`: The route gate's denial for a native read leads with the unlock step.
  - Behaviour: `hooks/route_gate.py` names the path, the scout tool to call first, and the number of seconds the unlock lasts, then gives the reason read is denied.
  - Outcome: The unlock step is visible even when a host shows only the first part of the message.
- `bash-gate-shell-structure`: The bash gate classifies commands inside shell structure and ignores text that is only data.
  - Behaviour: `src/command_policy.rs` classifies the command that follows `if`, `then`, `else`, `elif`, `while`, `until`, `do`, `{`, `!`, and `function NAME`. A heredoc body is dropped before classification unless its line feeds a shell or interpreter; `$(...)` inside any heredoc is still classified. A function defined in the same command is not reported as unrecognised, and a function name never overrides a command rule. `gh`, `mktemp`, and `pgrep` have default rules (`src/command_rules.rs`); `gh ... delete` warns.
  - Outcome: `if true; then rm -rf /; fi` and `{ rm -rf /; }` are blocked. Commit messages and file bodies passed through a heredoc no longer produce "unrecognised command" warnings.
- `bash-gate-alternatives`: A block that has a safe alternative names it.
  - Behaviour: The block reasons for a computed command name, `eval` of computed code, code piped into a shell, a recursive delete of an unresolved path, a force push, and a git config change each end with what to do instead. The closing sentence is "If it is intended, ask the user to run it."
  - Outcome: The agent can correct the command from the message alone.
- `gate-tests-isolated-home`: The bash-gate integration tests write gate events under a temporary `TEKMOR_HOME`.
  - Behaviour: `tests/bash_gate.rs` sets `TEKMOR_HOME` for every spawned `bash-gate`, and one test asserts the event lands in that home's `logs/gate-events.jsonl`.
  - Outcome: Running the test suite no longer adds block and warn events to the user's real gate log.
- `stop-gate-nearest-feature`: The stop gate suggests an owner for unclaimed code.
  - Behaviour: `gap` in `src/feature_map.rs` adds `(nearest feature: .verify/features/<file>)` to each unclaimed path, choosing the feature whose `## Code` entry shares the most leading path components. A path that shares none gets no suggestion.
  - Outcome: The block names the feature file to edit instead of only listing paths.
- `verification-from-checks-only`: Only `cli` and `orchestrator` runs count as verification.
  - Behaviour: `ran_a_check` in `src/verify.rs` gates `last_verification`, `suggest_verified_recipes`, and the success rate in `verify report`. Hook-observed shell commands are still stored but are not offered as recipes or reported as the last verification. A recipe needs a keyword match or execution intent; source alone no longer qualifies it.
  - Outcome: A route no longer recommends a past `git add` or `ls` as a "verified recipe".
- `bounded-read-count`: The stats count only bounded-read tools as token-tool calls.
  - Behaviour: `tool_log_stats_since` in `src/db/store.rs` counts `read_file_head`, `read_file_tail`, `grep_search`, and `file_summary`. Logged `bash`, `edit`, `write`, and `cursor_read` rows are excluded from the count.
  - Outcome: "Token tools: N bounded-read calls" matches the reader rows in `tool_log`.
- `suggested-tool-brief`: A suggested upstream tool carries a one-line description.
  - Behaviour: `brief` in `src/upstream/suggest.rs` keeps the first non-empty line of the tool description, cut to 160 characters. The full schema stays available through `get_mcp`.
  - Outcome: A route response no longer embeds multi-paragraph upstream tool descriptions.
- `constraints-sent-once`: A `must_apply` entry is sent in full once per MCP session, then named in one line.
  - Behaviour: `collapse_repeats` in `src/mcp/sent_constraints.rs` keeps entries whose topic and text were not sent in the last 30 minutes, and replaces the rest with one `still-applies` entry that lists their topics. A changed text counts as new. The list is never emptied while constraints apply, so the host hook still sees them as active.
  - Outcome: A repeat route carries one line for unchanged constraints instead of every rule again.
- `route-response-tokens`: `route_task` reports the size of the whole response.
  - Behaviour: The MCP handler in `src/mcp/server.rs` sets `response_tokens` to the token estimate of the serialized response. `tokens_used` still counts only the budgeted context.
  - Outcome: The cost of a route to the agent can be read from the response instead of inferred from the budget.
- `route-rule-dedupe`: A rule indexed from several paths is returned once, and the bundle does not repeat rule text.
  - Behaviour: `build_route_response` in `src/engine.rs` skips a rule whose topic and text are already in `applicable_rules`. `build_context_bundle` in `src/bridge.rs` clears `text` on `team_rules`, as it already does for skill text; the text stays in `applicable_rules`.
  - Outcome: The same `AGENTS.md` installed for three hosts no longer takes three rule slots.
- `route-stats-exclude-upstream`: Route latency and cache-hit rate ignore upstream MCP calls.
  - Behaviour: `retrieval_stats_since` in `src/db/store.rs` counts latency and cache hits only for rows whose phase is not `upstream_call`, and divides cache hits by route calls.
  - Outcome: A five-second upstream tool call no longer shows up as a slow route.

- `stop-gate-handback`: After the block limit, the stop gate hands the turn back with a NOT VERIFIED line.
  - Behaviour: `evaluate_stop` in `src/stop_gate.rs` returns `StopOutcome::HandBack` once `decide` allows the stop after `STOP_BLOCK_LIMIT` blocks. `hook_output` prints a `systemMessage` with no `decision` key, naming the repo, the first line of the reason, and `tekmor verify receipt`. `gate_event` records `stop-handback`.
  - Outcome: A turn no longer ends silently unverified on hosts that show `systemMessage`.
- `stop-gate-verified-line`: A turn that ended on a passing check on the current code prints one VERIFIED line.
  - Behaviour: `StopOutcome::Verified` from `evaluate_stop` in `src/stop_gate.rs` yields a `systemMessage` naming the repo, the passing command and the first twelve characters of the code hash, and pointing at `tekmor verify receipt`. No gate event is written.
  - Outcome: The user reads one line to judge a turn that changed code.
- `stop-gate-watch-mode`: Watch mode reports would-be stop blocks without blocking.
  - Behaviour: `GateMode` in `src/settings.rs` selects the mode, set by `gates.stop` in the settings file or by `TEKMOR_STOP_GATE=watch`. In watch mode `evaluate_stop` returns `HandBack` without writing block state, and the event is `stop-would-block`. The default is `enforce`.
  - Outcome: A new install can see what the gate would do before it enforces anything.
- `verify-init-force-blocked`: The agent cannot rewrite the allowlist through the binary.
  - Behaviour: `src/command_policy.rs` blocks `tekmor verify init` when `--force` is present. Plain `verify init` stays allowed because it never overwrites an existing file. Cases are in `src/command_policy_cases.txt`.
  - Outcome: The binary cannot bypass the Bash guard on `.verify/verify.toml`.

- `spec-requirements-receipt`: The receipt reports each requirement in `.verify/specs/*.md`.
  - Behaviour: `src/spec.rs` parses `R<n>.` lines with an optional backticked `check:` command. `receipt_for_project` in `src/receipt.rs` marks each requirement proven, failed, not run, no check yet, or not allowlisted.
  - Outcome: The receipt says "N of M requirements proven" instead of only that a command passed.
- `protected-checks` (removed): The agent may change `.verify/verify.toml`, `.verify/run.sh` and `.verify/checks/` through Bash or an edit tool. No Bash or edit guard blocks them, and the protected-path CI workflow is gone.
  - Behaviour: The receipt still lists these files when they differ from the last commit (`src/receipt.rs`). Use that line in review to see a changed allowlist or check script.
  - Outcome: Changes to checks are visible in the receipt, not blocked.
- `read-gate-large-files`: The read gate denies only large files that have not been scouted.
  - Behaviour: `check_read_tool_gate` in `hooks/route_gate.py` no longer denies a read because `must_apply` is active. The 64 KB threshold is unchanged.
  - Outcome: Small files are read and edited without an unlock call.
- `route-reuse-by-checkout`: A route is reused across directory changes inside one repository.
  - Behaviour: `session_route_reusable` in `hooks/route_gate.py` compares `checkout_key`, the nearest ancestor with `.git`, instead of the raw directory.
  - Outcome: A `cd` inside the repository no longer forces a new `route_task`.
- `verify-init-scaffold`: `init_verify` allowlists `.verify/run.sh` and no longer guesses commands.
  - Behaviour: `init_verify` in `src/verify.rs` lists the detected test commands and then `.verify/run.sh`. The Node runner comes from the lockfile (pnpm, yarn, bun, or npm). Deno, Maven, Gradle and mix projects are detected too. A Gemfile that names rspec gives `bundle exec rspec`; a Gemfile without rspec gives no test command. It no longer adds `.verify/` as a command or fills in four guessed commands when nothing is detected.
  - Outcome: The allowlist holds only commands the repository has, plus the verification entry point.

- `doctor-verify-allowlist`: Doctor says when the current repository has no verification allowlist.
  - Behaviour: `run` in `src/doctor.rs` prints `verify allowlist: missing — run: tekmor verify init` when `config::find_repo_root` finds a repository whose `.verify/verify.toml` does not exist.
  - Outcome: A new repository gets its first check set up from one command.

- `index-rule-scope`: Only `.mdc` files, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and files under a `rules` directory are indexed as rules. Test, fixture, example, spec and docs directories inside installed packages are not indexed.
  - Behaviour: `is_rule_file` and `is_package_support_dir` in `src/index.rs`. `is_noise_rule_source` makes `prune_noise_rule_rows` drop stored rules that no longer qualify on the next sync. `prune_removes_stored_rules_that_are_not_rule_files` in `src/db/store.rs` covers it.
  - Outcome: Fixture READMEs and package docs no longer come back as applicable rules.
- `package-duplicates`: Doctor reports installed packages that ship identical skill text.
  - Coverage: `index.rs` line coverage is 94% across all test targets, including `tests/index_sync.rs`, which runs `sync_index_opts` against a temp HOME and checks that unchanged files are skipped on the second run. `packages/mod.rs` line coverage is 92%.
  - Behaviour: `duplicate_packages` in `src/packages/mod.rs` digests the `SKILL.md` files of each package under `packages/`. `run` in `src/doctor.rs` prints one line per duplicate pair with the `package remove` command.
  - Outcome: A package installed twice under two names is visible, and nothing is removed automatically.
- `lessons-from-fixes`: A passing check that follows a failing run of the same command stores one lesson for the changed code.
  - Behaviour: `learn_from_fix` in `src/lessons.rs` is called from `record_from_result` in `src/verify.rs` for `cli` and `orchestrator` runs only. The excerpt is the first failure-marker line of the failing run's saved output, or its last line. Lessons are keyed by project, command and paths, expire after 60 days, and are skipped when the excerpt looks like a secret. `route_lessons` returns up to three lessons that touch an open file or a changed code path. Migration v27 adds the `lessons` table. A lesson keeps the paths changed at the failing run that are still changed at the passing run; each verify row stores its changed code paths in `changed_paths_json` (migration v28).
  - Outcome: The next route that touches the files a fix changed shows the earlier failure and its excerpt.
- `file-context`: A route names the owning feature and up to two allowlisted verification commands for up to five open files.
  - Behaviour: `route_file_context` and `parse_verification_commands` in `src/feature_map.rs`, called from the `route_task` handler in `src/mcp/server.rs`. Files with no owner use the nearest feature, as `nearest_owner` does for unclaimed code. The response field is `file_context`.
  - Outcome: The agent sees which feature owns a file and which command proves it, without searching.
- `feedback-real-checks-only`: Only `cli` and `orchestrator` runs credit or debit routed items.
  - Behaviour: The attribution block in `record_from_result` (`src/verify.rs`) is gated on `ran_checks`. Hook-observed commands record no context feedback. Only items whose source path the agent read in the same route are credited (`opened_paths_for_route` in `src/db/store.rs`, read from `tool_log`).
  - Outcome: Routed items are no longer credited for an unrelated `ls` or `grep`.
- `mcp-tool-surface`: `mcp.tool_surface: core` in the settings file lists twelve core tools.
  - Behaviour: `list_tools` in `src/mcp/server.rs` filters `CORE_TOOLS` when the setting is `core`; the default `full` lists every tool. `call_tool` is unchanged, so a hidden tool still runs when called by name. `McpSettings` and `ToolSurface` are in `src/settings.rs`.
  - Outcome: A smaller tool list for agents that choose by name, with no change for hosts that call hidden tools.

- **H1/H2 completion (status, doctor, verify init, lessons, duplicate packages):** `status` and `doctor` print the stop-gate mode (`GateMode::as_str`, `src/settings.rs`). `init_verify` allowlists nothing for a project with no recognised test runner, and `tekmor verify init` then prints "no test command detected; add yours to .verify/verify.toml". The VERIFIED line names the command and a 12-character code hash on both proof paths (`proof_line`, `src/stop_gate.rs`). `run_gc` deletes lessons past their 60-day TTL and reports `lessons_removed` (`prune_expired_lessons`, `src/lessons.rs`). `doctor` prints a removal hint per duplicate package (`duplicate_package_lines`, `src/doctor.rs`). `gates.stop` is documented in `config.example.yaml`.
  - Outcome: Tests: `init_verify_in_an_unrecognised_project_allowlists_nothing`, `init_verify_detects_a_makefile_as_a_test_runner`, `a_passing_check_on_changed_code_ends_the_turn_with_a_receipt_line` (command and hash), `lessons_past_the_ttl_are_pruned_and_recent_ones_are_kept`, `two_packages_with_the_same_skills_get_a_removal_hint`, and the settings tests for `gates.stop`.

- `route-budget-line`: The route briefing reports routed tokens against the budget. It no longer shows an estimated savings figure (R-9).
  - Behaviour: `format_budget_line` in `tekmor/src/route_briefing.rs` shows the index size and routed tokens. The retrieval log no longer writes `saved_pct`. Stats no longer report `total_saved_tokens`, `avg_saved_pct` or `routes_with_savings`. The dashboard headline counts tool savings only.
  - Outcome: No route output or stats figure rests on an estimate.

- `recipe-execution-intent`: Verified recipes are offered only for requests that ask to run something (R-4).
  - Behaviour: `suggest_verified_recipes` in `tekmor/src/verify.rs` returns nothing without execution intent. The route no longer adds a `verified-recipe` must-apply entry (`tekmor/src/engine.rs`).
  - Outcome: Routes for non-execution tasks carry no recipe and no recipe must-apply entry.

- `retrieval-stats-dedupe`: Memory feedback in a route lists each topic once (R-17).
  - Behaviour: `tekmor/src/engine.rs` skips a memory whose topic is already listed in `retrieval_stats`.
  - Outcome: A topic that matches two memories appears once in the route response.

## Code

- `tekmor/src`
- `tekmor/tests`
- `tekmor/hooks`
- `tekmor/Cargo.toml`
- `tekmor/config.example.yaml`
- `.cursor`
- `Cargo.lock`
- `README.md`
- `LICENSE`

## How to get to it (user POV)

- CLI: `tekmor verify`, `verify init`, `verify report`, `memory put`, `install --global`, `doctor`, `briefing`, `sync`
- MCP: `route_task`, `store_memory`, `verify_run`, `record_verify_outcome`, `file_summary`, `grep_search`, `read_file_head`, `read_file_tail`

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test -p tekmor-core` (passes 11 of 11, see `.verify/artifacts/feature-map-baseline/core-unit.log`). The whole crate runs with `cargo test --all-features -- --skip graphify_1k_ci_gate --skip scale_1k_passes_gate --test-threads=1`, the allowlisted command.
- **Targeted tests for the memory and graphify fixes:** `cargo test -p tekmor --all-features --lib -- store_and_delete_memory_round_trip concurrent_identical_stores_persist process::tests` (5 passed when run in this pass).
- **Recall-eval tests:** `cargo test -p tekmor --features extras --lib eval` (5 passed in this pass). Live check: `tekmor eval --ci --live`, which needs the cached all-MiniLM-L6-v2 model.
- **Stop-gate tests:** `cargo test -p tekmor --lib stop_gate` covers the scope, the code-tree hash, the block limit, and the matrix of owned and unclaimed code.
- **Feature-graph and feedback-report tests:** `cargo test -p tekmor --lib feature_graph`, `cargo test -p tekmor --lib feedback_report`, and `cargo test -p tekmor --bin tekmor cli_help`.
- **Functional API Tests (Hurl):** not yet written for the CLI or MCP surface.
- **Allowlisted Verification:** `.verify/run.sh`, listed in `.verify/verify.toml`
- **Preconditions:** Rust toolchain. Memory tests need a temporary `brain.db` path, not the user's `~/.autonomic/memory`.

### Verification Steps

1. **core-unit-check** (sub-feature: route-task-context)
   - Request: `cargo test -p tekmor-core`
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
- The hook tests in `hooks/test_route_gate.py` run inside `.verify/playbooks/verify_multi_agent_enforcement.sh` with `python3 tekmor/hooks/test_route_gate.py`, which `.verify/run.sh` calls. A passing `.verify/run.sh` therefore covers the Python hook.
- Plain `cargo test` in this crate does not build the `extras` tests. Any command that bypasses the allowlist must pass `--features extras` or `--all-features`.
- `doctor --fix` clears quarantine. Run it only on this repository's build output, not on a user's installed binary.
- Memory tests must use a temporary database path. Writing to the real `brain.db` changes shared memory.
- graphify subprocesses run through `graphify/process.rs` `output_with_timeout`. A graph build job is killed after 30 minutes and a `query_codebase` call after 60 seconds, so a hung graphify no longer holds `JOB_LOCK` or the MCP request forever. `graphify hook install` in `graphify/repos.rs` still runs with no timeout.
- Version: 1.0.0-rc5, set in `tekmor/Cargo.toml`. `Cargo.lock` is the workspace lock and is listed in the Code section. See CHANGELOG.md for the release contents.
- Stop hook binary: the Stop hook runs `~/.local/bin/tekmor`, which is installed separately from the release build. Until the rebuilt binary is installed, the gate runs the old scope rules (any change needs a run), and runs recorded by the old binary have no `code_hash`, so they do not count.
- Stop gate scope: only code paths need a passing run, and a run counts only for the code tree it tested (`code_hash`, from `src/verify.rs` `code_tree_hash`). Doc-only changes get the feature-map and knowledge-base owner check. The gate does not parse feature-graph frontmatter, because frontmatter is optional.
- Flaky under parallel tests: `sessions::digest::tests::ingests_gemini_jsonl_digest` failed once in a parallel `cargo test --all-features --lib` run (left 3, right 1). It passes under the allowlisted `--test-threads=1` run, which exits 0 (recorded by `tekmor verify`). Cause not investigated.
- Fixed: `sessions::digest::tests::distills_session_trajectories_into_procedure_edges` failed in parallel runs because it and `ingests_gemini_jsonl_digest` both set the process-wide `TEKMOR_SESSION_HOME`. Both now take a shared test-only lock (`SESSION_HOME_ENV` in `src/sessions/digest.rs`). The full `--lib` suite passed three runs in a row.
- Log writers take structs: `observability::log_native_tool_use(store, NativeToolUse { .. })` and `observability::log_upstream_call(store, UpstreamCall { .. })`. The MCP server builds them at each call site.

- Formatting pass (rustfmt, 2026-11-07): line breaks and spacing only, no behaviour change.
- Help coverage: every top-level command that `main.rs` dispatches has a `cli_help.rs` entry, including `report`, `status`, `bootstrap`, `rules`, `ext`, `memorable`, and the four hook entry points. Without an entry, `tekmor <cmd> --help` prints "Unknown command" even though the command runs.
