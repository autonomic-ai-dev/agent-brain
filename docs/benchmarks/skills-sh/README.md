# skills.sh production-scale eval

[skills.sh](https://skills.sh) catalogs **730k+** agent skills (Vercel). agent-brain cannot index the full catalog in every CI run, so we:

1. **Commit a snapshot** of real skills (`snapshot.json`) synced via public APIs
2. **Pad with filler skills** to **2000 items** — simulates a crowded production index
3. **Gate routing** with golden queries (`golden.json`) at **Recall@3 ≥ 0.80**

## CI

- **Every push:** `stage-skills-sh-eval.yml` runs `eval --skills-sh` (no network; uses committed snapshot)
- **Weekly / manual:** optional `sync` job refreshes snapshot from skills.sh (rate-limited; use `workflow_dispatch`)

## Commands

```bash
# Gate (2000-item simulated index)
cargo run --release -p agent-brain -- eval --skills-sh --write docs/benchmarks/skills-sh-latest.json

# Refresh snapshot (respects skills.sh rate limits; start with --required-only)
cargo run --release -p agent-brain -- skills-sh sync --required-only --write docs/benchmarks/skills-sh/snapshot.json
cargo run --release -p agent-brain -- skills-sh sync --max 100 --delay-ms 3000 --write docs/benchmarks/skills-sh/snapshot.json
```

## APIs used

| Endpoint | Auth | Purpose |
|----------|------|---------|
| `GET /api/search?q=…` | Public | Discover skill ids |
| `GET /api/download/{source}/{slug}` | Public | Fetch SKILL.md bodies |
| `GET /api/v1/skills/…` | Vercel OIDC | Full catalog (optional; not required for CI) |

## Files

| File | Role |
|------|------|
| `manifest.json` | Required ids + discovery queries + max snapshot size |
| `snapshot.json` | Committed skill bodies (indexed text) |
| `golden.json` | Queries + expected skill topics |
| `../skills-sh-latest.json` | Last eval report (CI artifact) |

## Growing the snapshot

1. Add skill ids to `manifest.json` `required_ids` (from skills.sh search)
2. Add matching cases to `golden.json`
3. Run `skills-sh sync` with generous `--delay-ms` (skills.sh returns 429 if too fast)
4. Run `eval --skills-sh` locally before pushing

See [../../architecture/13-proofs-and-benchmarks.md](../../architecture/13-proofs-and-benchmarks.md).
