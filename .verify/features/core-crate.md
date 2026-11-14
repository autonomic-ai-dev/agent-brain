---
feature: core-crate
knowledge_base: "[[../knowledge-base/core-crate]]"
related:
  - "[[cli-proof]]"
---

# core-crate

Summary: `tekmor-core` holds the pure proof types and the gate rule dispatcher. It has no storage, runtime or IO dependencies.

## Sub-features

- `core-evidence`: The record a receipt reads and when it proves a head and tree.
  - Behaviour: `EvidenceRecord`, `is_run_for` and `summarize` in `crates/tekmor-core/src/evidence.rs`.
  - Outcome: Receipts and summaries use one record type.
- `core-gate-rule`: One verdict shape for every gate.
  - Behaviour: `GateRule` and `evaluate` in `crates/tekmor-core/src/gate_rule.rs`. A block beats a warning.
  - Outcome: The bash and commit gates return the same verdict type.
- `core-verify-types`: The two verify types the CLI reports.
  - Behaviour: `VerifiedRecipe` and `VerificationStatus` in `crates/tekmor-core/src/types.rs`.
  - Outcome: Shared shapes for verify output.
- `core-boundary`: The core crate depends on no storage, runtime or IO crate.
  - Behaviour: `.verify/checks/core_boundary.sh` checks the normal dependency tree against a denylist.
  - Outcome: The script prints `core boundary holds`.

## Code

- `tekmor/crates/tekmor-core`

## How to get to it (user POV)

- Used through the `tekmor` CLI. There is no separate command.

## Verification Suites (Targeted Test Commands)

- **Unit tests:** `cargo test -p tekmor-core`
- **Static check:** `.verify/checks/core_boundary.sh`

## Gotchas

- The boundary check is a denylist of crate names. Extend it when a new IO dependency is added.
