# skills.sh production-scale eval

[skills.sh](https://skills.sh) catalogs **730k+** agent skills (Vercel). agent-brain cannot index the full catalog in every CI run, so we:

1. **Commit a snapshot** of real skills (`snapshot.json`) synced via public APIs
2. **Build `fixture-2k.db`** — pre-indexed SQLite with **2000 real** skills.sh skills (no synthetic fillers)
3. **Gate routing** with golden queries (`golden.json`) at **Recall@3 ≥ 0.80** — currently **50 cases**, **1.00** on the committed fixture (see `../skills-sh-latest.json`).

CI opens the committed DB (copied to a temp dir) — no runtime seeding.

**Index composition:** 2000 real skills.sh skills (`source_path LIKE 'https://skills.sh/%'`). Verify with `fixture verify` (expect `bench_filler_rows: 0`).

## CI

- **Every push:** `stage-skills-sh-eval.yml` runs `eval --skills-sh` against `fixture-2k.db`
- **Weekly / manual:** optional `sync` job refreshes snapshot, rebuilds fixture DB

## Commands

```bash
# Build / refresh committed fixture DB (after snapshot changes)
cargo run --release -p agent-brain -- fixture build --write docs/benchmarks/fixture-2k.db
cargo run --release -p agent-brain -- fixture verify --db docs/benchmarks/fixture-2k.db

# Gate (uses fixture-2k.db by default when present)
cargo run --release -p agent-brain -- eval --skills-sh --write docs/benchmarks/skills-sh-latest.json

# Compare runtime seed vs committed DB
cargo run --release -p agent-brain -- eval --skills-sh --seed

# Expand golden cases (probes fixture routing; writes golden.json)
cargo run --release -p agent-brain -- skills-sh golden-probe --target 30

# Refresh snapshot (rate-limited; ~20–40 min for 2000 skills)
cargo run --release -p agent-brain -- skills-sh sync --target 2000 --merge --delay-ms 400 --write docs/benchmarks/skills-sh/snapshot.json

# Upgrade metadata-only entries to full SKILL.md bodies (rate-limited)
cargo run --release -p agent-brain -- skills-sh retry-failed --write docs/benchmarks/skills-sh/snapshot.json --delay-ms 200
# then: fixture build && eval --skills-sh
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
| `snapshot.json` | Skill bodies (source for `fixture build`) |
| `golden.json` | Queries + expected skill topics |
| `../fixture-2k.db` | Pre-indexed 2000-skill benchmark DB (committed) |
| `../skills-sh-latest.json` | Last eval report (CI artifact) |

## Growing the snapshot

1. Run `skills-sh sync --target 2000 --merge` (checkpoints to `snapshot.json` every 50 skills)
2. Run `fixture build` then `eval --skills-sh` locally before pushing
3. Run `skills-sh golden-probe --target 30` to add routing cases that pass on the fixture
4. Add skill ids to `manifest.json` `required_ids` if they must always be present

See [../../architecture/13-proofs-and-benchmarks.md](../../architecture/13-proofs-and-benchmarks.md).
