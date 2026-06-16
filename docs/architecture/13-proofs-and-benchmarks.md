# 13 — Proofs and benchmarks

Architecture docs cite latency and accuracy targets. This article separates **what CI proves** from **what is aspirational** and how to reproduce results.

## Summary

| Claim category | CI proof? | How to verify |
|----------------|-----------|---------------|
| Recall@3 ≥ 0.85 (memory + skills) | **Yes** | `agent-brain proofs --ci` |
| Turn-cache p95 ≤ 30 ms (500-skill fixture) | **Yes** | same |
| Warm-route p95 ≤ 100 ms (fixture, deterministic embed) | **Yes** | same |
| Production brain Recall@3 | **No** | `eval --ci --live` (informational) |
| skills.sh snapshot Recall@3 (2000-item index) | **Yes** | `eval --skills-sh` — see [`skills-sh/`](../benchmarks/skills-sh/README.md) |
| &lt;50 ms p95 on real ONNX + full index | **No** (nightly informational) | `bench --onnx --fixture-db` → [`onnx-latest.json`](../benchmarks/onnx-latest.json) |
| Hook latency &lt;1 ms | **Yes** (gate logic) | `python3 agent-brain/hooks/test_route_gate.py` in CI |

Published artifact: [`docs/benchmarks/latest.json`](../benchmarks/latest.json)

## Isolated fixture (why not production brain.db)

Early `eval --ci` used `~/.agent_brain/data/brain.db`. On populated machines, golden skills competed with thousands of real indexed items — **Recall@3 failed** while isolated tests passed.

**Fix:** CI and published proofs use:

- Temp directory via `Config::isolated`
- `Embedder::deterministic()` (hash-based unit vectors)
- Seeded golden items + 491 filler skills (`bench-filler-####`)

Production routing quality on your ECC library is a **separate operational concern** — extend golden cases for your skill pack or run `eval --live` after major index changes.

## Commands

```bash
# CI gate + write published JSON
cargo run --release -p agent-brain -- proofs --ci --write docs/benchmarks/latest.json

# Accuracy only (isolated)
cargo run --release -p agent-brain -- eval --ci

# Accuracy on your real brain (may fail — not a CI gate)
cargo run --release -p agent-brain -- eval --ci --live

# Latency only (isolated)
cargo run --release -p agent-brain -- bench --ci

# ONNX warm-route on committed fixture-2k.db (nightly / local)
cargo run --release -p agent-brain -- bench --onnx --write docs/benchmarks/onnx-latest.json

# Hook gate latency (CI)
python3 agent-brain/hooks/test_route_gate.py

# skills.sh catalog (committed snapshot + 2000 fillers)
cargo run --release -p agent-brain -- fixture build --write docs/benchmarks/fixture-2k.db
cargo run --release -p agent-brain -- eval --skills-sh --write docs/benchmarks/skills-sh-latest.json

# Criterion (local, not CI)
cargo bench -p agent-brain --bench route_task
```

## ProofReport schema (`latest.json`)

```json
{
  "generated_at": "RFC3339 timestamp",
  "environment": "isolated-fixture",
  "embedder": "deterministic",
  "fixture_skills": 500,
  "eval": { "memory": { ... }, "skills": { ... } },
  "latency": {
    "turn_cache_hit": { "p95_ms": 0, ... },
    "warm_route": { "p95_ms": 5, ... },
    "passed": true
  },
  "passed": true
}
```

## Latency methodology

1. Seed 500 skills (golden + decoys + fillers)
2. Warmup: 5 unique `route_task` calls
3. **Turn-cache hit:** 25 identical queries → record `latency_ms` p50/p95
4. **Warm route:** 25 unique queries (embedder warm, turn cache miss) → p50/p95
5. Assert p95 against thresholds in `bench.rs`

Deterministic embedder makes CI stable; **real ONNX** routes are slower — see `latency_ms` in MCP logs for production measurements.

## Accuracy methodology

Golden suites in `eval.rs`:

- 4 memory queries → expect topic in top-3 memory
- 6 skill queries → expect topic in top-3 skills
- 3 decoy skills seeded to catch false positives

Threshold: `RECALL_AT_3_THRESHOLD = 0.85` per suite.

## What we explicitly do not gate

| Claim | Reason |
|-------|--------|
| Cold ONNX first embed (seconds) | Dominated by model load; install-time concern |
| Full user index routing | Fixture-only for reproducibility |
| Cross-host enforcement | Not measurable in Rust CI |
| LLM reranker accuracy | Not implemented |

## For principal architects

- Treat **`proofs --ci` green** as contract: routing logic + fixture index behave as specified.
- Treat **architecture latency tables** as design targets unless labeled **Proven (CI)**.
- Require **custom golden cases** when adopting proprietary skill packs — default goldens do not cover your library.
- Use **`eval --live`** periodically on a staging machine with production index — informational, not blocking.

## Further reading

- [12-routing-accuracy.md](12-routing-accuracy.md)
- [10-concurrency-and-performance.md](10-concurrency-and-performance.md)
- [../benchmarks/README.md](../benchmarks/README.md)
