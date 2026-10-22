---
name: feature-map
description: "Create, maintain, and verify a repository's feature map in .verify/: behaviour contracts linked to unit and functional test suites, with step-level verification, coverage checks, and an evidence record. Use for any repository that keeps a .verify/ harness, or that needs one."
disable-model-invocation: true
---

# Feature map

A feature map is the repository's maintained inventory of behaviour. It links what users and operators can do to the tests that prove it. Its reader is any coding agent or contributor who has not seen the codebase before, reading the file in the middle of a task.

A good map answers three questions for that reader:

1. Which behaviours exist, and how to reach each one.
2. Which exact test commands prove a change to a behaviour.
3. Which traps, gaps, and stale expectations apply before anyone edits the code.

## Load the reference you need

| Task | Load |
|---|---|
| Write or review a feature contract | `references/contract-format.md` |
| Discover the surfaces, write the index, or apply the authoring rules | `references/authoring-rules.md` |
| Set up `.verify/`, run the quality gates, or record a review | `references/layout-and-gates.md` |
| Check that every test entry is covered by exactly one step | `scripts/feature_coverage.sh` |
| Check a JaCoCo coverage floor | `scripts/coverage_gate.sh` |
| Run the ordered gates and write the evidence record | `scripts/gate_report.sh` |

## Core invariants

- Every feature contract has exactly four `H2` sections, in order: Sub-features, How to get to it, Verification Suites, Gotchas.
- Every step gives its request, its reason, its expected result with values, and its impact.
- Every functional test entry is covered by exactly one step.
- Calendar-dependent and environment-dependent values come from a recorded live query. Values not confirmed live are marked `[UNCONFIRMED]`.
- Commands contain no angle-bracket placeholders.
- Failing tests are fixed, not weakened. A gap or a failure is recorded in Gotchas.
- Every verification command is run through the allowlist in `.verify/verify.toml`.
