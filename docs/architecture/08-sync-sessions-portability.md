# 8. Sync, sessions, and portability

## Summary

agent-brain targets **one brain per developer** across machines. Git and encrypted cloud sync move SQLite bundles; session ingest optionally turns IDE chat history into digest facts.

## What we built

### Sync bundles

Export/import JSON bundles containing facts and metadata (not a raw copy-paste of `brain.db` while live). Entry points:

- `agent-brain export` / `import`
- `agent-brain sync git {init,push,pull}`
- `agent-brain sync cloud {push,pull}` — S3-compatible via OpenDAL

**Merge policies:** `newer_wins`, etc. (`sync/bundle.rs`)

### Git sync (`sync/git.rs`)

- Bare remote or file URL workflow
- Push exports bundle to git remote; pull imports on second machine
- Uses write queue on pull to avoid corrupting active DB

### Cloud sync (`sync/cloud.rs`)

- Encrypts bundle with key from `AGENT_BRAIN_SYNC_KEY` or keychain ref in config
- Providers: S3, R2, MinIO, local FS for tests

**Why encryption:** Brain DB contains project paths and conventions — treat as sensitive.

### Session ingest (`sessions/`)

Discovers user messages from:

| Source | Location |
|--------|----------|
| Cursor | Agent transcripts under user home |
| Codex | Codex session paths |
| Gemini / Antigravity | `~/.gemini/**/transcript.jsonl` |
| OpenCode | `opencode.db` SQLite |

**Digest pipeline:** Extract user messages → optional LLM-free summarization → `store_fact` with `source: session_digest` and topic `session-digest-{source}-{slug}`.

**Background:** `AGENT_BRAIN_SESSION_INGEST_BG` runs after bootstrap delay.

**Why digests, not full import:** Keeps memory atomic and router-sized; avoids storing assistant tool dumps.

### Scope conflicts

`scope_conflict_warnings` in route response when global vs project facts disagree on same topic — surfaces ambiguity to the agent.

## Alternatives considered

### Live replicate `brain.db` via Dropbox/iCloud

**Rejected:** SQLite WAL + concurrent write corruption risk; bundle export is point-in-time safe.

### CRDT / real-time multi-device sync

**Deferred:** Complexity far exceeds solo-dev use case; git pull model is enough for laptop + desktop.

### Central sync server (agent-brain cloud)

**Rejected:** Same objections as cloud router API — privacy and ops.

### Ingest assistant messages too

**Mostly rejected:** Noisy, tool-json heavy; user messages carry intent signal.

### Single `session-digest-cursor` topic (legacy)

**Removed:** Colliding topics overwrote each other; per-session topics prevent clobbering.

## Trade-offs

- **Sync is not automatic realtime** — user or cron must `git push` / `cloud push`.
- **Session ingest** can add low-signal facts — GC archives stale session digests.
- **Conflict resolution** is policy-based, not semantic merge of fact text.

## For senior engineers and principal architects

### Why bundle export instead of live DB replication

SQLite + cloud sync folders (Dropbox/iCloud) is a **known corruption pattern** — WAL files and concurrent writers from two machines. Bundle export/import is **point-in-time**, idempotent, and goes through the write queue on import.

Think **eventual consistency with explicit operator push** — like `git push`, not Redis pub/sub.

### Encryption on cloud path only

Local `brain.db` is plaintext on disk. Cloud sync encrypts bundles because object storage is a **higher trust boundary** than a developer laptop (shared buckets, backup policies). PEs with full-disk encryption still get defense-in-depth on S3/R2.

### Session ingest philosophy

We ingest **user messages**, not assistant tool dumps, because:

- User text carries **intent** (“use vitest”, “fix the PR CI”)
- Assistant messages are huge, repetitive, and tool-json heavy
- Digest facts are **router-sized** atomic statements

Session ingest is **off by default path** in many setups — treat as opt-in automation with GC safety net.

### Per-session digest topics

Legacy single topic `session-digest-cursor` caused **clobbering** — new session overwrote old. Per-session topics trade storage for correctness. GC archives stale digests so the fact table does not grow forever.

### Merge policies

`newer_wins` is the default because brains are **single-writer per machine**; conflicts are rare and usually timestamp-skew. Semantic merge of fact text (“use pnpm” vs “use npm”) is **not attempted** — humans resolve via `scope_conflict_warnings` + edit/delete.

### Failure modes

| Failure | Risk | Mitigation |
|---------|------|------------|
| Import during active session | Queue delay, not corruption | Write queue |
| Duplicate facts after merge | Same topic, two scopes | Scope rules + conflict warnings |
| Stale laptop brain | Old conventions win with wrong policy | Document merge policy per team |
| Session ingest PII | Paths in user messages | GC + avoid ingest on sensitive repos |

### Questions a PE should ask

1. Is **git-based sync** acceptable for your security team vs cloud bucket?
2. What **merge policy** applies when two machines edited the same topic?
3. Should **session ingest** be disabled org-wide?
4. Do you need **selective sync** (facts only, no logs)? (Customize export today.)

## Further reading

- [14-portable-sync.md](14-portable-sync.md) — portable sync upgrade (thin slice)
- [sync/mod.rs](../../agent-brain/src/sync/mod.rs)
- [sessions/mod.rs](../../agent-brain/src/sessions/mod.rs)
- [08-sync-sessions-portability.md](08-sync-sessions-portability.md) — this doc
