# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
