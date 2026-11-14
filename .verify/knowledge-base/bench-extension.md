---
feature: "[[../features/bench-extension]]"
---

# Bench extension

## What it does

`tekmor-bench` is the benchmark and CI-proof harness. It was moved out of the core binary into an optional extension, so the core stays small. It provides `bench` (CI, MCP, graph, scale, supervisor, ONNX, and BEAM suites) and `proofs`.

## How it works

1. The core binary has no `bench` or `proofs` commands. An unknown command is matched against the manifests of installed extensions (`extensions/manifests/*.json`).
2. A match runs `tekmor-bench` with the full argument list, including the subcommand, and with `TEKMOR_HOME` and `TEKMOR_SOCKET` set.
3. The extension's `cli::dispatch` handles `bench` and `proofs`. Each suite runs its own isolated engine and writes its report.

## Where the code is

- Crate: `extensions/bench` (`src/lib.rs`, `src/main.rs`, `src/cli.rs`, and the benchmark modules)
- Graph benchmark: `src/graphify_bench.rs` calls `tekmor_graphify::ingest::ingest_graph_at_path`, so bench depends on `extensions/graphify` (see [[graphify-extension]]).
- Moved here from core: `src/eval.rs` (Recall@3 gates), `src/fixture.rs` (fixture DBs), `src/skills_sh.rs` (skills.sh snapshot). The core `eval` gate test is now `extensions/bench/tests/eval_gate.rs`.
- Commands moved here: `fixture` and `skills-sh` (in `src/cli.rs`, with `eval`, `bench`, and `proofs`)
- Route calls: `bench.rs`, `beam_eval.rs`, `eval.rs`, `graphify_bench.rs`, `skills_sh.rs`, and `supervisor_bench.rs` build an `tekmor::engine::RouteRequest` for each `route_task` call. The struct replaced the seven positional arguments, with no change to what each benchmark measures.
- Integration tests: `extensions/bench/tests/v0_23.rs`, `extensions/bench/tests/v0_24.rs`
- Workspace member: `Cargo.toml` at the repo root
- Dispatch and install: `tekmor/src/extensions.rs`, `src/main.rs` (the fallback arm)
- Core support: `tekmor/src/db/store.rs` (`bm25_fast_path_eligible`), `tekmor/src/stats.rs` (`persist_proof_snapshot` moved out)

## How to use it

- `tekmor ext add bench`
- `tekmor bench --ci`, `tekmor bench --beam`, `tekmor proofs --ci --write PATH`
- `tekmor eval --ci` and `tekmor eval --ci --live` run the recall gate. Before the extension moved this command, `eval` was in the core binary and `--live` seeded the user's live store. The core binary now has no `eval` command, and `eval` routes to the installed extension.

## How it is verified

- `cargo test -p tekmor-bench --all-features`
- The allowlisted workspace suite through `tekmor verify`

## Known limits
- Merged with master PR #156: the bench layout on this branch is the one kept. Master bench changes were path-only and are preserved as in-crate `crate::` paths.

- The release binary is about 44 MB, since it links the core with `extras`.
- Install trust is the manifest SHA-256 plus the Developer ID team check (see [[cli-verify-and-memory]]).

## Related

- Feature map: [[../features/bench-extension]]
- Extension mechanism: [[cli-verify-and-memory]]

- Formatting pass (rustfmt, 2026-11-07): line breaks and spacing only, no behaviour change.

## Eval fixture location

- The three fixture files moved from `tekmor/eval/` to `extensions/bench/eval/` in the Horizon 2 change, where the loader already expected them.
- The skill pool for the eval is `eval/skills.jsonl`: 120 skills exported from the brain database (`skill` rows of at most 600 characters, ordered by topic). Regenerate it from a read-only query of the database; the export is not refreshed automatically.
- Review of the 28 eval cases: kept cases have a skill, rule or memory in the corpus that could answer them. The real-model run was worse on this fixture, so the deterministic embedder stays. The four misses return memory, not skills; they are recorded, not tuned.

## Estimated savings removed (R-9)

- The supervisor bench gates on routes within their token budget, from measured `tokens_used` and `tokens_budget`. The estimated savings gate was removed.
