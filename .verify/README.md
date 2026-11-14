# .verify

The verification harness for this repo.

- `verify.toml` — the allowlist of commands that `tekmor verify -- <cmd>` may run.
- `run.sh` — the full suite: check, core boundary, core tests, and the binary tests.
- `checks/` — `core_boundary.sh` (the core crate has no IO dependencies) and `require_tests.sh`.
- `features/` — feature maps: which code each feature owns and which checks prove it.
- `knowledge-base/` — one note per feature, linked from its map.
- `artifacts/` — run logs (not tracked).

Record a run with `tekmor verify -- .verify/run.sh`. Do not record agent-asserted results; only allowlisted runs count.
