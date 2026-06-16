# skills.sh production-scale eval

[skills.sh](https://skills.sh) catalogs **730k+** agent skills (Vercel). agent-brain cannot index the full catalog in every CI run, so we:

1. **Commit a snapshot** of real skills (`snapshot.json`) synced via public APIs
2. **Build `fixture-2k.db`** — pre-indexed SQLite with snapshot skills + fillers to **2000 items**
3. **Gate routing** with golden queries (`golden.json`) at **Recall@3 ≥ 0.80**

CI opens the committed DB (copied to a temp dir) — no runtime seeding.

**Index composition:** 3 real skills.sh skills + 1997 synthetic `bench-filler-*` skills = 2000 rows. Verify with `fixture verify`.

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

# Refresh snapshot (rate-limited)
cargo run --release -p agent-brain -- skills-sh sync --required-only --write docs/benchmarks/skills-sh/snapshot.json
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

1. Add skill ids to `manifest.json` `required_ids`
2. Add matching cases to `golden.json`
3. Run `skills-sh sync` with generous `--delay-ms`
4. Run `fixture build` then `eval --skills-sh` locally before pushing

See [../../architecture/13-proofs-and-benchmarks.md](../../architecture/13-proofs-and-benchmarks.md).
