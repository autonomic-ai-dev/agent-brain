# Feature contract format

Load this file when you write or review a `.verify/features/<feature>.md` contract.

## Structure

Every feature contract has, in this order:

1. An `H1` title naming the user-facing feature.
2. A summary: exactly one paragraph describing the external behaviour from the consumer's or operator's point of view.
3. Exactly four `H2` sections, in this order:

| # | Heading | Holds |
|---|---|---|
| 1 | `## Sub-features` | Atomic kebab-case identifiers, each with a behaviour, an intent, and an outcome |
| 2 | `## How to get to it (user POV)` | Entry points: CLI, HTTP, messages, and the local base address |
| 3 | `## Verification Suites (Targeted Test Commands)` | Unit command, functional command, allowlisted command, preconditions, and the numbered steps |
| 4 | `## Gotchas` | Traps, stale expectations, coverage gaps, and known failures |

No other `H2` sections are allowed.

## Sub-features

Each sub-feature is a bullet with a kebab-case identifier, then three nested lines:

```markdown
- `jobs-submit`: Validates and enqueues a job payload.
  - Intent: why the behaviour exists for the caller.
  - Behaviour: what it checks and does.
  - Outcome: what the caller or the system sees afterwards.
```

## Entry points

List every way a user, operator, or client reaches the feature. Give the literal method, path, command, or message name, and the local address when there is one.

```markdown
- HTTP: `POST /api/v1/jobs` with body `{"name": "...", "priority": "..."}`
- CLI: `jobctl push --name <name>` (replace the placeholder with a literal value in real commands)
```

Placeholders are for prose only. Commands in the contract use literal values or shell variables.

## Verification suites

This section names the exact commands and then lists the steps.

```markdown
- **Unit Tests:** the command that covers this feature's logic.
- **Functional API Tests (Hurl):** the command that runs `functional/<feature>.hurl`.
- **Allowlisted Verification:** the same runner command, as listed in the allowlist.
- **Preconditions:** services that must run, data that must exist, and the environment variables required.
```

### Four-part step records

Each step states:

- **Request:** the method, path or command, and the payload file.
- **Why:** the behaviour the step proves, in one sentence.
- **Expected:** the status and assertions the test entry checks, with values.
- **Impact:** what changes in the final state, and what a failure at this step means.

An Expected line must match an assertion in the test entry. A step with no matching assertion is invalid.

### Entry coverage

- Steps are numbered in the order of the functional file's request entries.
- Each step heading carries `(entries …)` with the entry numbers it covers.
- A step may cover a contiguous range of entries that share one purpose, and it names the range.
- Every entry is covered by exactly one step. The coverage script in `scripts/feature_coverage.sh` checks this.

Example heading: `3. **accept-all-future** (entries 26–30) — every installment falls after the cutoff date.`

## Gotchas

Gotchas hold what a reader must know before trusting the contract:

- Operational traps and timing requirements.
- Stale or unconfirmed expectations, and the evidence that shows them.
- Coverage gaps: what the suite does not test. A passing run never implies coverage of a listed gap.
- Known failures and their causes, with the decision that is still open.
- Concatenation effects: if several test files were merged, say that captured values now cross former file boundaries.
