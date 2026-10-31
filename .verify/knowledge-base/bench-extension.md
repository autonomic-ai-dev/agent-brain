---
feature: "[[../features/bench-extension]]"
---

# Bench extension

## What it does

`agent-brain-bench` is the benchmark and CI-proof harness. It was moved out of the core binary into an optional extension, so the core stays small. It provides `bench` (CI, MCP, graph, scale, supervisor, ONNX, and BEAM suites) and `proofs`.

## How it works

1. The core binary has no `bench` or `proofs` commands. An unknown command is matched against the manifests of installed extensions (`extensions/manifests/*.json`).
2. A match runs `agent-brain-bench` with the full argument list, including the subcommand, and with `AGENT_BRAIN_HOME` and `AGENT_BRAIN_SOCKET` set.
3. The extension's `cli::dispatch` handles `bench` and `proofs`. Each suite runs its own isolated engine and writes its report.

## Where the code is

- Crate: `extensions/bench` (`src/lib.rs`, `src/main.rs`, `src/cli.rs`, and the benchmark modules)
- Moved here from core: `src/eval.rs` (Recall@3 gates), `src/fixture.rs` (fixture DBs), `src/skills_sh.rs` (skills.sh snapshot). The core `eval` gate test is now `extensions/bench/tests/eval_gate.rs`.
- Commands moved here: `fixture` and `skills-sh` (in `src/cli.rs`, with `eval`, `bench`, and `proofs`)
- Integration tests: `extensions/bench/tests/v0_23.rs`, `extensions/bench/tests/v0_24.rs`
- Workspace member: `Cargo.toml` at the repo root
- Dispatch and install: `agent-brain/src/extensions.rs`, `src/main.rs` (the fallback arm)
- Core support: `agent-brain/src/db/store.rs` (`bm25_fast_path_eligible`), `agent-brain/src/stats.rs` (`persist_proof_snapshot` moved out)

## How to use it

- `agent-brain ext add bench`
- `agent-brain bench --ci`, `agent-brain bench --beam`, `agent-brain proofs --ci --write PATH`
- `agent-brain eval --ci` and `agent-brain eval --ci --live` run the recall gate. Before the extension moved this command, `eval` was in the core binary and `--live` seeded the user's live store. The core binary now has no `eval` command, and `eval` routes to the installed extension.

## How it is verified

- `cargo test -p agent-brain-bench --all-features`
- The allowlisted workspace suite through `agent-brain verify`

## Known limits

- The release binary is about 44 MB, since it links the core with `extras`.
- Install trust is the manifest SHA-256 plus the Developer ID team check (see [[cli-verify-and-memory]]).

## Related

- Feature map: [[../features/bench-extension]]
- Extension mechanism: [[cli-verify-and-memory]]
