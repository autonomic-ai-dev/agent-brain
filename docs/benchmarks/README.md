# Published proofs and benchmarks

This directory holds **reproducible proof artifacts** for claims in the architecture docs. Numbers here are generated from **isolated fixture databases** — not your production `~/.agent_brain/data/brain.db`.

## What is proven in CI

| Gate | Command | Threshold | Fixture |
|------|---------|-----------|---------|
| Memory Recall@3 | `proofs --ci` | ≥ 0.85 | 4 golden facts |
| Skills Recall@3 | `proofs --ci` | ≥ 0.85 | 6 golden skills + 3 decoys |
| Turn-cache p95 | `proofs --ci` | ≤ 30 ms | 500-skill index, deterministic embedder |
| Warm-route p95 | `proofs --ci` | ≤ 100 ms | Unique queries, embedder warm |

CI runs: `cargo run --release -p agent-brain -- proofs --ci --write docs/benchmarks/latest.json`

Integration test: `proofs_ci_passes_isolated_gates` in `agent-brain/tests/v0_10.rs`

## Regenerate locally

```bash
cargo run --release -p agent-brain -- proofs --ci --write docs/benchmarks/latest.json
```

Optional deeper benchmarks (Criterion HTML reports):

```bash
cargo bench -p agent-brain --bench route_task
# open target/criterion/report/index.html
```

## Claim types in architecture docs

| Label in docs | Meaning |
|---------------|---------|
| **Proven (CI)** | Gated in `stage-test.yml` via `proofs --ci` |
| **Measured (local)** | Instrumented (`retrieval_log`, `latency_ms`) — varies by machine and index size |
| **Design target** | North-star SLO (e.g. &lt;50 ms warm p95 on real ONNX) — not CI-gated |
| **Design rationale** | Engineering argument without automated proof |

Production `brain.db` with thousands of real skills is **not** part of the CI proof — use `eval --ci --live` to spot-check your machine only.

## skills.sh catalog (730k+ skills)

Separate workflow **`stage-skills-sh-eval.yml`** gates routing against **`fixture-2k.db`** — a committed pre-indexed DB of **2000 real** skills.sh skills.

| Gate | Command | Threshold |
|------|---------|-----------|
| skills.sh Recall@3 | `eval --skills-sh` | ≥ 0.80 on golden cases | **50/50 (1.00)** on 2000-skill fixture |

Index composition: **2000** real skills.sh rows, **0** `bench-filler-*` (`fixture verify`).

Rebuild after snapshot changes: `fixture build --write docs/benchmarks/fixture-2k.db`

See [skills-sh/README.md](skills-sh/README.md). Artifacts: `fixture-2k.db`, `skills-sh-latest.json`, `onnx-latest.json`.

## Files

| File | Contents |
|------|----------|
| `latest.json` | Last generated `ProofReport` (eval + latency on isolated fixture) |
| `skills-sh-latest.json` | Last skills.sh eval report |
| `skills-sh/` | Manifest, golden cases, committed snapshot |

## Source modules

| Module | Role |
|--------|------|
| `agent-brain/src/fixture.rs` | Isolated temp DB + seed helpers |
| `agent-brain/src/eval.rs` | Golden Recall@3 suites |
| `agent-brain/src/bench.rs` | Latency percentiles + thresholds |
| `agent-brain/src/proofs.rs` | Combined gate + JSON export |
| `agent-brain/src/skills_sh.rs` | skills.sh sync + 2000-item eval gate |
| `agent-brain/benches/route_task.rs` | Criterion micro-benchmarks |

See also [../architecture/13-proofs-and-benchmarks.md](../architecture/13-proofs-and-benchmarks.md).
