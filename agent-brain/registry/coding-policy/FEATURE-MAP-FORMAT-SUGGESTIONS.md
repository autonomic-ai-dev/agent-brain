# Feature map and verification suites: format rules

Scope: generic rules for any repository that keeps a `.verify/` harness. These rules are applied to `AGENTS.md` section 16 and to `.cursor/skills/feature-map/SKILL.md`. Platform copies of the policy need the same section 16 text (see Sync).

## Why these rules exist

Converting a repository to this format exposed gaps the skill did not cover:

- A feature contract that lists only sub-feature names and entry points does not say what a step should prove, so a failing step cannot be classified as a defect or as a stale expectation.
- A value copied from a local config file went stale against the live system. Fixtures built on the stale value produced many false failures. A single live query settled it.
- A step named a boundary case, but its payload did not contain the boundary value. The contract and the fixture disagreed and nothing flagged it.
- Two entries in one run shared a request identifier, so one create silently returned another entity.
- The contract's step list and the test file's entry list drifted apart.

## Rules

### 1. Four-part step records

Each step in `## Verification Suites` states, in order:

- Request: method, path, and the payload file.
- Why: the behaviour the step proves, in one sentence.
- Expected: the status and the assertions the test entry checks, with values.
- Impact: what changes in the final state (data, events, external calls, or nothing), and what a failure at this step means.

A step whose Expected line is not backed by an assertion in the test entry is invalid.

### 2. Step-to-entry linkage

- Number steps in the order of the test file's entries.
- Every request entry is covered by exactly one step. A step may cover a contiguous range of entries that share one purpose; it names the range, and its Expected line states what each entry asserts. Enforce coverage with the bundled script (`scripts/feature_coverage.sh` in the feature-map skill), run from the repository's static phase. It reads the `(entries …)` ranges from the feature contract and fails on an uncovered or doubly covered entry.
- Every payload a step names must exist in `functional/payloads/`.

### 3. Sourced expectations

- Values that depend on the environment's reference data (calendar dates, period or cycle identifiers, derived amounts) come from a recorded live query. Record the query, its result, and the date in the feature file.
- Values derived from code name the class or method.
- Values not confirmed by a live run are marked `[UNCONFIRMED]`.
- A live value that contradicts a local config file wins. Update the config, rebase the fixtures built on it, and record the change in Gotchas.

### 4. Fixtures test what their step says

- A payload used for a boundary step contains the boundary value the step names. For example, a step named for an exact cutoff boundary needs a record dated exactly on the cutoff.
- Name payloads for what they test. Use the same meaning for "happy", "past", and "future" across the repository.

### 5. Unique request identifiers per run

- Within one run, every request identifier is unique across all feature files that share a run identifier.
- Give each feature a suffix scheme and state it in `features/README.md`.

### 6. Named coverage gaps

- Gotchas lists what the suite does not test. A passing run never implies coverage of a listed gap.

### 7. Concatenated test files

- If several test files are merged into one feature file, Gotchas states that captured values now cross the former file boundaries, and that a run must check for entries that now read a value they should not see.

### 8. Layout

```text
.verify/
├── verify.toml        # allowlisted commands
├── run.sh             # phases: doctor, unit, static, functional; argument = feature or all
├── doctor.sh          # read-only checks: tools, ports, token presence (value never printed)
├── features/
│   ├── README.md      # preconditions, driving conventions, proof standards, feature index
│   └── <feature>.md   # four H2 sections, step records per rule 1
├── functional/
│   ├── <feature>.hurl
│   └── payloads/      # rendered into artifacts/<RUN_ID>/ by run.sh
├── checks/            # generic harness checks (coverage script)
├── suites/            # repository-specific checks and helpers
├── env/               # test variables; each live value records its source
└── artifacts/         # gitignored; logs and reports
```

### 9. No angle-bracket placeholders

- Commands use shell variables (`$RUN_ID`, `"$AUTH_TOKEN"`) or literal names. Angle-bracket placeholders are not allowed in commands or in template names inside prose.

## Sync

- Section 16 text is synchronized across all platform copies (AGENTS.md, claude/CLAUDE.md, antigravity/AGENTS.md, opencode/AGENTS.md, augment/AGENTS.md).

## Open roadmap items

- Ship a step-count check as a reusable `doctor.sh` fragment.
- Decide whether a repository should keep its own copy of these rules in its `docs/`.
