---
feature: bench-extension
knowledge_base: "[[../knowledge-base/bench-extension]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[core-crate]]"
---

# bench-extension

Summary: The benchmark and CI-proof harness, shipped as the optional `agent-brain-bench` extension instead of inside the core binary. It provides the `bench` and `proofs` subcommands.

## Sub-features

- `bench-commands`: Runs the benchmark suites and their gates.
  - Behaviour: `agent-brain bench --ci | --mcp | --graphify | --scale | --supervisor | --onnx | --beam`, each with `--assert` and `--write PATH`.
  - Outcome: A JSON report on stdout, and an exit code from the gate.
- `proofs-command`: Aggregates the CI proofs.
  - Behaviour: `agent-brain proofs --ci [--write PATH]`. It also writes `metrics/proof-latest.json` under the brain home.
  - Outcome: The aggregate report, and a nonzero exit when a proof fails.

## Code

- `extensions/bench`
- `Cargo.toml`

## How to get to it (user POV)

- `agent-brain ext add bench`, then `agent-brain bench --ci` and `agent-brain proofs --ci`

## Verification Suites (Targeted Test Commands)

- **Extension tests:** `cargo test -p agent-brain-bench --all-features` (the lib tests and the `v0_23` and `v0_24` integration tests).
- **Allowlisted:** `.verify/run.sh`, which runs the workspace suite including this crate.

## Gotchas

- The extension links the whole core library with `extras`, so its release binary is about as large as the core binary (44 MB at rc5). Installing it adds that much.
- `eval --beam` moved to `bench --beam`. Scripts that call the old form now get the eval usage text.
- The core exposes `bm25_fast_path_eligible` on the store for the supervisor benchmark. Changing that method breaks the extension.
