# Feature Map & Verification Contract Format Specification

Scope: Canonical format rules for any repository utilizing the `.verify/` verification harness under `agstack`.

---

## 1. Directory Structure (`.verify/`)

```text
.verify/
├── verify.toml        # Allowlisted commands and default verify step
├── run.sh             # Phased composite runner: doctor, unit, static, functional
├── doctor.sh          # Read-only health check (ports, binaries, token presence)
├── features/
│   ├── README.md      # Index table, baseline preconditions, driving conventions
│   └── <feature>.md   # 4-H2 feature contracts with four-part step verification
├── functional/
│   ├── <feature>.hurl # Declarative Hurl scenario tests
│   └── payloads/      # JSON/fixture payloads referenced by four-part steps
├── checks/            # Generic harness static checks (feature_coverage.sh, etc.)
├── suites/            # Repository-specific test helpers and suites
├── env/               # Environment variables (each live value records its source)
└── artifacts/         # Gitignored output directory for logs, reports, and evidence
```

---

## 2. Strict 4-H2 Feature Contract Format

Every `.verify/features/<feature>.md` MUST enforce exactly four H2 sections in this order:

### H2 1: `## Sub-features`
Atomic kebab-case identifiers pairing each sub-feature with intent, behavior, and outcome:
```markdown
## Sub-features

- `jobs-submit`: Validates and enqueues job payloads.
  - Intent: Allow clients to submit asynchronous batch jobs.
  - Behaviour: Validates payload schema, assigns a UUID, and enqueues to NATS.
  - Outcome: HTTP 201 response with job_id and queued status in DB.
- `jobs-retry`: Retries failed jobs up to max_attempts.
  - Intent: Prevent transient failures from permanently abandoning work.
  - Behaviour: Increments attempt counter with exponential backoff and jitter.
  - Outcome: Job moves to running state or dead-letter queue after max attempts.
```

### H2 2: `## How to get to it (user POV)`
Concrete entry points available to a user, operator, or external client:
```markdown
## How to get to it (user POV)

- CLI: `jobctl submit --payload <path>`
- HTTP: `POST /api/v1/jobs` with JSON payload
- NATS: Subject `jobs.inbound` with JSON payload
```

### H2 3: `## Verification Suites (Targeted Test Commands)`
Explicit targeted commands discovered from codebase navigation, followed by numbered four-part step records:
```markdown
## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test --package jobs-worker --lib`
- **Functional API Tests (Hurl):** `hurl --test .verify/functional/jobs.hurl`
- **Allowlisted Verification:** `sh .verify/run.sh jobs`
- **Preconditions:** Service running on `127.0.0.1:8080`, tenant seeded.

### Verification Steps

1. **submit-valid-job** (entry 1)
   - Request: `POST /api/v1/jobs` using payload `.verify/functional/payloads/jobs/valid-job.json`
   - Why: Proves the endpoint accepts well-formed jobs and returns 201.
   - Expected: HTTP 201; JSON body contains `status: "queued"` and non-empty `id`.
   - Impact: Enqueues job record; failure means job submission is broken.

2. **query-queued-job** (entry 2)
   - Request: `GET /api/v1/jobs/{{job_id}}`
   - Why: Proves job status can be retrieved immediately after creation.
   - Expected: HTTP 200; `status` equals `queued`.
   - Impact: Verifies read-after-write consistency.
```

### H2 4: `## Gotchas`
Operational traps, timing requirements, coverage gaps, and unconfirmed expectations:
```markdown
## Gotchas

- Debounce timing: Allow up to 100ms before asserting state transitions from queued to running.
- Coverage gap: Does not test network partitions between worker and DB.
- Unconfirmed expectation: Max payload size is assumed to be 10MB per config, [UNCONFIRMED] live.
```

---

## 3. Mandatory Invariants

1. **Four-Part Step Records:** Each step states: `Request:`, `Why:`, `Expected:` (with concrete assertions), and `Impact:`.
2. **1:1 Step-to-Entry Linkage:** Every request in `.hurl` files must be mapped to exactly one step heading carrying `(entries …)`.
3. **Sourced Expectations:** Reference data derived from live environments records the query and date. Derived from code names the class. Unconfirmed values are marked `[UNCONFIRMED]`.
4. **Boundary Fixtures Match Steps:** Fixtures used for boundary steps must contain the exact boundary values named.
5. **Zero Placeholders:** Never use `<token>` or `<id>`. Use shell variables (`$PORT`) or literal names.
6. **Unique Request IDs:** Every request ID is unique across feature runs.
