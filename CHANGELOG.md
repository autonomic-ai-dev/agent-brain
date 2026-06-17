# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.14.0] - 2026-06-17

### Added

- **`suggested_native_tools` in `route_task`** — steers agents to bounded-read MCP tools (`grep_search`, `file_summary`, `read_file_head`, `read_file_tail`) before full `Read`.
- **Token tool telemetry** — `tool_log` table (schema v9), hook event ingest, and `stats` reporting for tool calls, token savings, and inefficient Read steers.
- **Read gate hooks** — `AGENT_BRAIN_READ_GATE` (`steer`|`hard`|`off`) blocks or steers Cursor `Read` on large/blocked paths when `must_apply` is active.
- **`apply_when` query path matching** — `path:**/dist/**` can match from user message text, not only open files.
- **`agent-brain suggest-memory`** — surfaces hook-captured anti-pattern suggestions in briefing.
- **Linux ARM64 release binary** — CI publishes `agent-brain-aarch64-unknown-linux-gnu` (fixes `update --force` 404 on aarch64 Linux).

### Changed

- **Auto-update** — validates release assets via GitHub API before download; clearer errors when platform binary is missing.
- **`install.sh`** — better 404 guidance for Linux ARM64 before v0.14.0.

## [0.13.0] - 2026-06-17

### Added

- **Token-efficient MCP tools** — `grep_search`, `file_summary`, `read_file_head`, `read_file_tail` with per-response token savings metadata.
- **Blocked path guard** — refuses `dist/`, `node_modules/`, `target/`, `build/` unless `allow_blocked_paths=true`.
- **Token tools bench** — gated in `proofs --ci` / `bench --supervisor` (≥ 80% savings vs full read on 2k-line fixture).

### Changed

- **`@supervisor` pack** — skills and rule now direct agents to agent-brain bounded-read tools before Cursor Read/cat.
- **MCP server instructions** — mention token-efficient file tools explicitly.

## [0.12.0] - 2026-06-17

### Added

- **Bundled `@supervisor` package** — execution-supervisor rule + token-efficient-ops / execution-supervisor skills.
- **Supervisor routing** — `must_apply` pre-scan, negative-memory pinning, supervisor lexical boost, BM25 fast-path for supervisor queries.
- **`bench --supervisor`** — skill recall, must_apply hit rate, token savings, latency gate.
- **Proofs CI** — supervisor bench embedded in `proofs --ci`; writes `supervisor-latest.json`.

### Changed

- **Briefing / stats** — supervisor constraint counts in route summary and operator metrics.

## [0.11.0] - 2026-06-16

### Added

- **Memory GC reporting** — `agent-brain memory gc` JSON now includes `reason_buckets` (archive vs protected breakdown) and `top_topics` (most frequent candidate topics).
- **Configurable GC thresholds** — `--stale-days` / `--very-stale-days` CLI flags and `memory_gc` block in `~/.agent_brain/config.yaml`.

### Changed

- **Integration tests** — `Engine::new_with_store` uses deterministic embeddings to avoid fastembed ONNX lock contention in parallel `cargo test`.

### Fixed

- **CI flake** — parallel test runs no longer contend on fastembed model cache locks.

## [0.10.1] - 2026-06-16

### Changed

- **Dependency security patch** — upgraded `rmcp` to `1.7.0` and `lru` to `0.16.4` to resolve Dependabot alerts.
- **rmcp compatibility** — updated non-exhaustive struct construction and upstream tool call request building for `rmcp` v1 API.
- **README updates** — added v0.10 operator loop (`promote`, `memory gc`, `digest --weekly`, `eval --ci`) and current multi-host installer commands.

## [0.10.0] - 2026-06-16

### Added

- **`promote_to_skill` MCP tool** — stages SKILL.md drafts from memory facts under `~/.agent_brain/staging/`.
- **`agent-brain promote list|approve|reject`** — human approval gate before skills land in `.cursor/skills/`.
- **Memory GC** — `agent-brain memory gc [--apply] [--force]` archives stale facts to `facts_archive` using `context_weights`; protects negative/`apply_when`/high-confidence user facts unless `--force`.
- **Operator digest** — `agent-brain digest --weekly` summarizes retrieval_log and context feedback for the past 7 days.
- **Eval CI gate** — `agent-brain eval --ci` runs golden Recall@3 eval (threshold ≥ 0.85).
- **Schema v6** — `skill_staging` and `facts_archive` tables.

## [0.9.1] - 2026-06-16

### Added

- **OpenCode installer** — `agent-brain install --opencode [--global]` writes `mcp.agent-brain` into `opencode.json` (project root or `~/.config/opencode/opencode.json`) using OpenCode's `type: local` + `command` array format.
- **OpenCode instructions** — `agent-brain.md` at `.opencode/` (project) or `~/.config/opencode/` (user) on first install.
- **`--all`** now includes OpenCode user config.

## [0.9.0] - 2026-06-16

### Added

- **Multi-host installers** — `agent-brain install --claude-desktop`, `--vscode [--global]`, `--claude-code [--global]`, `--all`.
- **`host_install` module** — per-host config paths, JSON merge (Cursor/Claude `mcpServers`, VS Code `servers`).
- **Claude Code rule template** — `~/.claude/agent-brain.md` or `.claude/agent-brain.md` on install.
- **`docs/host-integration.md`** — host-agnostic guide, config locations, hook parity matrix, verification checklist.

### Changed

- `install` CLI defaults to Cursor; `--global` applies to Cursor or scopes VS Code / Claude Code to user profile.
- Install help text documents all host targets.

## [0.8.0] - 2026-06-16

### Added

- **Upstream MCP federation** — `upstream_mcp` block in `~/.agent_brain/config.yaml` registers up to two stdio MCP servers.
- **`route_to_mcp` MCP tool** — explicit upstream tool calls with semantic JSON/text truncation (`max_tokens` default 500).
- **`suggested_tools` in `route_task`** — keyword-ranked upstream tool hints from the indexed catalog.
- **Upstream tool index** — refreshed on bootstrap via `list_tools`; stored in `brain.db` meta.
- **Upstream call logging** — `retrieval_log` entries with `phase=upstream_call`.
- **Keychain env refs** — `${VAR}` in upstream `env` resolves via keychain/env and registers `secret_refs`.

### Changed

- `rmcp` client features enabled (`client`, `transport-child-process`) for upstream subprocess transport.

## [0.7.3] - 2026-06-16

### Added

- **Gemini / Antigravity session digests** — ingests `~/.gemini/**/transcript.jsonl` (`USER_INPUT` / `<USER_REQUEST>` parser).
- **OpenCode session digests** — reads user messages from `~/.local/share/opencode/opencode.db` (override with `AGENT_BRAIN_OPENCODE_DB`).
- **Per-session digest topics** — `session-digest-{source}-{slug}` so each conversation keeps its own fact (no more single colliding `session-digest-cursor` topic).
- **CLI** — `agent-brain sessions ingest [--source cursor,codex,gemini,opencode] [--legacy]` and `agent-brain sessions status`.
- **`AGENT_BRAIN_SESSION_HOME`** — override home directory scanned for session files (tests / custom layouts).

### Changed

- Session discovery unified across Cursor, Codex, Gemini, and OpenCode.
- Digest meta keys use `session_digest:{source}:{session_id}` instead of full file paths.

## [0.7.2] - 2026-06-16

### Added

- **Write-queue hardening** — `import_memory`, CLI `import`, `sync git pull`, and `sync cloud pull` serialize through the shared `Engine` write queue (`WriteOp::ImportBundle`).
- **`db/write_handler.rs`** — central write-thread handler for store, delete, and import ops.
- **`docs/concurrency.md`** — documents queued vs non-queued paths.
- **CLI MCP auto-approve** — `install --global` merges `agent-brain:*` into `~/.cursor/permissions.json` so Cursor CLI agents skip per-session MCP approval (requires Run Mode enabled).

### Changed

- `Engine` owns the write queue (MCP no longer spawns a second queue).
- `git_pull` / `cloud_pull` take `&Engine` instead of separate store/embedder args.

## [0.7.1] - 2026-06-16

### Added

- **Scoped route gate** — default `AGENT_BRAIN_ROUTE_GATE_SCOPE=brain_mcp` gates only agent-brain MCP tools; Shell/Read/Grep keep working when MCP is down (`all` restores legacy strict mode).
- **MCP offline mode** — disconnect failures set `mcp_offline_until` (default 30m, `AGENT_BRAIN_ROUTE_OFFLINE_SECS`); hooks stop hard-locking the session until `route_task` succeeds again.
- **`install --reload`** — bumps `AGENT_BRAIN_BUILD` in `mcp.json` to nudge Cursor to reload agent-brain without a full reinstall.
- **`serve_meta.json`** — written on `serve` start; **`doctor`** reports running vs on-disk version and flags stale serve.

### Changed

- Route gate clears offline state on successful `route_task`.
- `doctor --fix` runs `install --global --reload` when serve is stale.

## [0.7.0] - 2026-06-16

### Added

- **Sync S3 (cloud):** `sync cloud push|pull` with tar.zst bundle + age encryption (`AGENT_BRAIN_SYNC_KEY`); S3-compatible storage via opendal (AWS S3, R2, MinIO); `provider = "local"` for dev/tests.
- **Secret refs:** `secret_refs` table + `secret_refs.json` in sync bundles (names only, never values); `agent-brain secrets status|setup|add` with OS keychain storage.
- **`sync status` cloud fields:** `last_push`, `last_pull`, `artifact_present`.
- **`sync.cloud.auto_push`** — optional cloud push after `store_memory` (when `sync.cloud.enabled`).

### Changed

- Schema v5: `secret_refs` table.

## [0.6.1] - 2026-06-16

### Added

- **Route gate grace mode** — after a failed `route_task` (MCP disconnect), hooks allow other tools for 120s (`AGENT_BRAIN_ROUTE_GRACE_SECS`); stale gate auto-opens after 45s (`AGENT_BRAIN_ROUTE_STALE_SECS`).
- **Phase inference** — broader keyword sets for reviewing/planning/implementing when host omits `phase`.
- **Memory signal ranking** — penalize `legacy-*` and `session-digest-*` facts; exclude low-signal memories from extra candidate pool; cap to one low-signal memory in route output.
- **Briefing visibility** — one-line `briefing` now includes top skill/agent names.

### Changed

- Default MCP `RUST_LOG` from `agent_brain=warn` to `agent_brain=info` (Cursor no longer treats INFO as errors).

## [0.6.0] - 2026-06-15

### Added

- **Sync S2 (git):** `sync git init|clone|push|pull|status`; bundle at `~/.agent_brain/sync/bundle/`.
- **`sync status`** — git repo state + unresolved conflicts + recent conflict log entries.
- **`sync restore <id>`** — re-promote a fact superseded during sync import.
- **`sync cloud push|pull`** — S3 groundwork (config schema; implementation deferred).
- **`sync.git.auto_push`** — optional git push after successful `store_memory`.
- Git imports tag `conflict_log.sync_source=git`; schema v4 adds `conflict_log.restored`.

### Fixed

- Git sync commits set local `user.name` / `user.email` when missing (fixes CI and fresh machines without global git config).
- Release build warning: session digest word limit now applied; clippy cleanups across lib and tests.

## [0.5.0] - 2026-06-15

### Added

- **`apply_when` on facts** — phase/tag/path conditions with +0.15 score boost; matching facts surface in `must_apply`.
- **Convention confidence boost** — +0.08 for `source=user` or `confidence >= 0.95`.
- **`warnings` in `route_task`** — global vs project fact conflicts on the same topic.
- **Sync S1** — sync bundle export/import (`agent-brain export`, `agent-brain import`, MCP `import_memory`).
- Schema v3 migration: `facts.apply_when`.

## [0.4.0] - 2026-06-15

### Added

- **Session digests (B):** structured one-fact-per-transcript import; legacy snippet ingest behind `AGENT_BRAIN_SESSION_INGEST_LEGACY=1`.
- **Context intelligence (C):** optional `phase` on `route_task`, negative memory `polarity` → `must_apply`, phase-aware score boost, `report_context_useful` MCP tool, conflict log on topic supersession.
- **Operator visibility (D):** `retrieval_log` persistence, `explain_last_context` MCP tool, `agent-brain inspect log|fact|conflicts` CLI.
- Schema v2 migration: `facts.polarity`, `conflict_log`, `retrieval_log`, `context_weights`.

## [0.4.0-rc.1] - 2026-06-15

### Changed

- Cursor project rule template (`install --global`) now includes route briefing visibility and macOS codesign guidance so reinstalls do not strip those sections.
- MCP auto-update checks GitHub on every `serve` start (and every 15 minutes while running), independent of the 24h package `interval_hours`.
- Default MCP auto-update startup delay reduced from 300s to 60s.

### Added

- `make sync-release` / `scripts/sync-local-release.sh` — download latest release and link MCP immediately (`agent-brain update --force`).

## [0.3.15] - 2026-06-15

### Added

- `agent-brain doctor --fix`: adhoc re-sign macOS binaries, realign `mcp.json`, refresh hooks.
- macOS adhoc codesign after MCP binary auto-update and `install --global`.
- `make release-macos` / `scripts/build-release-macos.sh` for post-build adhoc sign on macOS.
- CI adhoc-signs macOS release artifacts; `install.sh` signs after download.

### Fixed

- macOS taskgated SIGKILL (`Code Signature Invalid`) on linker-signed Rust binaries used by Cursor MCP.

## [0.3.14] - 2026-06-15

### Added

- Human-readable route briefing: `~/.agent_brain/logs/last-route.md`, one-line `briefing` in `route_task`, stderr summary in MCP output.
- `agent-brain briefing` and `agent-brain doctor` CLI commands.

## [0.3.13] - 2026-06-15

### Added

- `agent-brain version` and `agent-brain --version` print the installed crate version.

### Changed

- Documentation for startup tuning env vars, idle-gated MCP restart settings in `config.yaml`, and `install --global` MCP defaults.
- `config.example.yaml` enables MCP auto-update by default (safe now that `v0.3.12+` is on GitHub).

## [0.3.12] - 2026-06-15

### Added

- Config-driven auto-update for installed packages and the MCP binary (`~/.agent_brain/config.yaml`).
- `agent-brain update [--force]` and `agent-brain config init|show` CLI commands.
- Background auto-update on MCP `serve` when `auto_update.enabled` is true.
- MCP self-update can auto-restart the `serve` process (`mcp.restart_after_update`, Unix `exec` + Cursor reconnect).
- Idle-gated MCP restart: waits until no in-flight tool calls plus `restart_idle_secs` quiet window (`mcp_activity`).
- Startup stagger env vars: `AGENT_BRAIN_BOOTSTRAP_DELAY_SEC`, `BOOTSTRAP_INTERVAL_SEC`, `AUTO_UPDATE_DELAY_SEC`, `SESSION_INGEST_DELAY_SEC`.
- Cursor project rule at `.cursor/rules/agent-brain.mdc` (replaces non-loading `~/.cursor/rules/` path).

### Fixed

- MCP `serve` no longer blocks on full index sync before stdio is live — Cursor enablement is much faster (`AGENT_BRAIN_BOOTSTRAP_BG=0` restores blocking bootstrap).
- Index sync skips re-embedding unchanged files (content-hash match) and only bumps `index_version` when items actually change.
- `postToolUse` hook clears route gate for Agent MCP tools (`mcp_agent-brain_route_task`).
- MCP auto-update skips download when local binary is ahead of GitHub latest (dev builds).
- In-place MCP restart only when `current_exe` matches `mcp.bin_path`.

### Changed

- Embedding model loads lazily on first embed (not at process start).
- Default branch renamed to `master`.
- `install --global` writes startup-tuned MCP env defaults and `RUST_LOG=agent_brain=warn`.

## [0.3.10] - 2026-06-15

### Fixed

- MCP `route_task` schema advertised `limits` defaults as all zeros; Cursor sent `{0,0,0,0}` and routing returned empty results. Schema and `RouteLimits::Default` now use agents=2, skills=3, rules=5, memory=5; all-zero limits are normalized at runtime.
- Empty `route_task` responses are no longer cached (and stale empty cache entries are ignored).

### Changed

- `build_route_response` fills agents, skills, and rules before memory so skills are not crowded out by legacy session memories within the token budget.
- Release pipeline uses `pipeline-compose-run`: tests must pass before cross-platform build and GitHub release publish on `v*` tags. Push/PR and tag entry workflows only dispatch stages declared in `.github/pipelines/`; stage workflows are `workflow_dispatch` only.
- Test stage runs a Linux snapshot build and uploads the binary before the cross-platform release build stage.
- Pipeline stages export context via `pipeline-compose-export` (`version`, `git_sha`, `snapshot_run_id`, `run_id`) and pass values to downstream stages through pipeline `context`.
- Workflow concurrency groups cancel superseded CI and stage runs per branch/PR; release publish stays non-cancellable mid-flight.

## [0.3.9] - 2026-06-15

### Fixed

- Integration tests use `Embedder::deterministic()` so CI does not download ONNX models from HuggingFace (avoids 429 rate limits)

## [0.3.8] - 2026-06-15

### Added

- Batched SIMD-friendly dot-product scoring for all FTS candidates in one pass
- Configurable embedding model via `AGENT_BRAIN_EMBED_MODEL` (`mini`, `fast`/`mini-q`, `bge-small`, `bge-small-q`); auto-clears stale vectors on model change

### Changed

- Query embedding cache and index vectors invalidate when the embedding model changes

## [0.3.7] - 2026-06-15

### Added

- BM25-only fast path skips query embedding when FTS matches are strong (`AGENT_BRAIN_BM25_FAST_PATH=0` to disable)
- Background session ingest on `serve` so MCP is live before legacy transcript import (`AGENT_BRAIN_SESSION_INGEST_BG=0` for sync)
- Scope-aware fallback candidate pools (project + global first, not all packages)
- Turn cache ignores open files by default for higher hit rate on multi-step loops (`AGENT_BRAIN_TURN_CACHE_OPEN_FILES=1` to include them)

### Changed

- `route_task` logs `bm25_fast_path=true` when ONNX embed is skipped

## [0.3.6] - 2026-06-15

### Added

- In-memory LRU query embedding cache (128 entries) atop SQLite persistence, with dual-key lookup via `fingerprint_query(user_message)`

### Changed

- Search index cache uses `Arc` snapshots — no full index clone per `route_task`
- BM25 prefilter runs in parallel with query embedding (overlapped wall time)
- Index and query embeddings stored as unit vectors; scoring uses dot product instead of per-candidate cosine

## [0.3.5] - 2026-06-15

### Added

- Bootstrap prewarm loads the in-memory search index and embedder model so the first `route_task` after MCP start is not cold (`AGENT_BRAIN_PREWARM=0` to disable)
- SQLite `query_embeddings` cache persists query vectors across restarts (`AGENT_BRAIN_EMBEDDING_CACHE=0` to disable)
- `route_task` structured latency logs (`embed_ms`, `score_ms`, `build_ms`, `candidates`, `index_total`, `embed_cache_hit`, `cache_warm`, rolling `p95_ms`) on stderr with `RUST_LOG=agent_brain=info`

### Changed

- BM25 prefilter + in-memory index cache scores FTS candidates only (fixes BM25 rowid/id bug; much faster on large indexes like ECC)
- Candidate lookup uses id-indexed maps instead of scanning the full index

## [0.3.4] - 2026-06-15

### Added

- **Cursor hooks enforcement** — `install --global` installs `route_gate.py` hooks that block tools until `route_task` succeeds each user turn
- Stronger MCP server instructions and Cursor rule for required `route_task` usage

## [0.3.3] - 2026-06-15

### Added

- Dedupe agents/skills by name in `route_task` and `get_context` (highest score wins when packages overlap)
- Purge indexed items from `brain.db` when a package is removed

## [0.3.2] - 2026-06-15

### Added

- **Hack (removable):** auto-ingest legacy Cursor/Codex chat transcripts into memory on startup (`AGENT_BRAIN_SESSION_INGEST=0` to disable)

### Changed

- GitHub Actions bumped to Node 24-native majors (`checkout@v6`, `upload-artifact@v7`, `download-artifact@v7`, `rust-cache@v2.9.1`, `action-gh-release@v3`)

## [0.3.1] - 2026-06-15

### Added

- `agent-brain add <owner/repo>` to install GitHub skill/agent packages (e.g. `affaan-m/ecc`)
- `agent-brain package list|update|remove` for package management
- Optional `agent-brain.yaml` manifest for custom package index roots
- [docs/USAGE.md](docs/USAGE.md) with setup, daily workflow, and MCP auto-start guide

### Changed

- GitHub Actions bumped to Node 24-native action majors (no `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` workaround)
- Release notes are generated from this changelog instead of auto-generated summaries
- README instructions expanded for first-time setup on a new machine

## [0.3.0] - 2026-06-15

### Added

- Phase 1 MCP server: `route_task`, `get_context`, `store_memory`, `list_memory`, `delete_memory`, `export_memory`
- Local indexing for agents, skills, rules, and memory from Cursor/Claude/Codex paths
- Turn cache (LRU, 60s TTL) and SQLite WAL write queue
- `agent-brain install` command to write Cursor `mcp.json`
- `scripts/install.sh` one-liner installer
- CI builds with GitHub Actions artifacts for macOS, Linux, and Windows
- Release workflow publishing platform binaries on `v*` tags
