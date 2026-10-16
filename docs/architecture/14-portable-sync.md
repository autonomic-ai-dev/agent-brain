# 14. Portable setup and memory sync

**Status:** thin slice implemented (phases 0–2 stacked). **Next:** phases 3–4 after baseline outcome data. **Backlog:** phases 5–9 until justified.

**Related:** [08-sync-sessions-portability.md](08-sync-sessions-portability.md).

## Phase 0 baseline (recorded)

Source: [docs/benchmarks/portable-sync-baseline.json](../benchmarks/portable-sync-baseline.json) (`generated_at` 2026-10-15T20:47:54Z, isolated fixture, deterministic embedder).

| Metric | Value | n |
|--------|-------|---|
| Recall@3 (combined) | 1.0 | 20 cases (7 memory + 13 skills) |
| Warm-route p95 | 2 ms | 25 samples |
| Turn-cache-hit p95 | 0 ms | 25 samples |
| proofs `--ci` | passed | — |

Later phases must not regress below baseline. Absolute Recall@3 ≥ 0.85 is already met.

## Naming: `scope` vs `domain`

| Term | Meaning | Values |
|------|---------|--------|
| **scope** | Memory visibility in the local DB | `global`, `project`, later `imported:<pack>` |
| **domain** | Sync destination | `personal`, `work`, or **unclassified** |

Domain is assigned **per `project_key`** in the device overlay. Unclassified projects are **local-only**.

## What already exists / gaps

See plan. Confirmed: path `scope_key`, fixed `source=agent`, export dumps all, `--ff-only` overwrite, no temporal/tombstones in bundle, plaintext git, one remote.

## Critical design

1. **Transactional outbox** — `sync_events` in same SQLite txn as fact writes; push flushes to `devices/<id>/events.jsonl`.
2. **Domain per project** — unclassified never pushed.
3. **Verifier** — `agent-brain verify -- <cmd>`, `verify record`, and MCP `record_verify_outcome`; project-root **`.verify/`** playbooks (agent-agnostic) auto-hashed into `scenario_dir_hash`; Shell PostToolUse auto-records matching test commands (`source=hook`). `agent-brain report` breaks down by `source` (`cli` strongest). Background MCP auto-update **stages** downloads and applies only when idle (no mid-session `mcp.json` bump).
4. **`min_reader_version`** on root manifest.
5. **Eval gate** — not below baseline; synced-fact golden tasks.

## Thin slice build order

0 Baseline → 1 project_key + outbox/v2 + git rebase → 1b minimal bootstrap → 2 verify/report → stop and reassess.

Phases 3–4 after thin slice proves out. Phases 5–9 backlog.

## Bundle v2 layout

```text
~/.agent_brain/sync/bundle/
  manifest.json                 # schema_version 2, min_reader_version, devices[]
  devices/<device_id>/
    manifest.json
    events.jsonl
```


## Project `.verify/` playbooks

Agent-independent verification playbooks live at **`<project-root>/.verify/`** (same level as `.git`). agent-brain discovers this marker by walking ancestors of cwd, hashes the tree into `scenario_dir_hash`, and records outcomes via:

- `agent-brain verify -- <cmd>` (`source=cli`)
- `agent-brain verify record --exit N --source playbook -- <summary>`
- MCP `record_verify_outcome` (after a playbook doctor/drive)
- Cursor Shell PostToolUse auto-record for common test commands (`source=hook`)

Suggested layout: `README.md` (Launch / Doctor / Drive / Evidence / Cleanup), `features/`, optional `scripts/`.

## Ownership

| Concern | Owner |
|---------|--------|
| Bundle v2, outbox, project_key, verify, report | agent-brain |
| Full marketplace plugin | Separate repo (after 1b) |
| Done-gating | agent-spine / harness |


## Thin slice delivery (stack #63)

Stacked PRs (bottom → top): docs baseline → project_key schema → wire routing → outbox/v2/git → bootstrap+verify.

Stop here and reassess before phases 3–4 (scratch/handoff) or 5–9 (domains, plugin, sharing, learning, risk).
