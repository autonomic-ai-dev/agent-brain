# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
