---
feature: "[[../features/core-crate]]"
---

# Core crate

## What it does

Version 1.0.0-rc5 (`tekmor/crates/tekmor-core/Cargo.toml`). See CHANGELOG.md for the release contents.

`tekmor-core` holds the pure routing logic of tekmor: the behavior-contract rules, retrieval ranking, query decomposition, and memory decay. It has no storage, runtime, or IO dependency, so the logic can be tested with no database, daemon, or network.

## How it works

1. **Behavior contract.** `behavior_contract` returns the rules every routed response carries. The must-apply rules and the search-before-asking and evidence rules are in its text.
2. **Retrieval ranking.** Reciprocal rank fusion (`retrieval_fusion`) favors items found by more than one source. An exact identifier match boosts a candidate. Cross-encoder reranking (`cross_encoder_rerank`) keeps a strong hybrid leader on top.
3. **Query decomposition.** `query_decompose` splits a compound query such as "X and also Y" into sub-queries, and keeps quoted phrases whole.
4. **Memory decay.** `memory_decay` lowers the weight of old, unreinforced memories. A higher useful count slows the decay. A fresh memory keeps full weight.
5. **Boundary.** `.verify/checks/core_boundary.sh` reads the normal dependency tree of the core crate and fails if it contains a storage, runtime, or IO crate.

## Where the code is

- Crate: `tekmor/crates/tekmor-core`
- Re-exports in `src/lib.rs` keep the old `tekmor::<module>` paths valid
- Boundary check: `tekmor/.verify/checks/core_boundary.sh`

## How to use it

- There is no separate user command. Running `tekmor route_task` from any repository uses this logic.
- Developers work on it through `cargo test -p tekmor-core`.

## How it is verified

- `cargo test -p tekmor-core`: 11 tests, all passing
- `sh .verify/checks/core_boundary.sh`: exits 0 with "core boundary holds"

## Known limits

- The boundary check is a denylist of crate names. A new IO crate that is not on the list passes. Extend the list when a new IO dependency is added.
- `commit_gate` reads stdin, so it stays in the adapter crate and is outside the core boundary.
- Removing a re-export in `lib.rs` breaks any caller that uses the old path.

## Related

- Feature map: [[../features/core-crate]]
- Routing and verify CLI: [cli-verify-and-memory](cli-verify-and-memory.md)
- `RouteTaskResponse` in `src/types.rs` gained `response_tokens: Option<usize>` on 2026-11-13. It is set by the MCP handler in the main crate, not by core. It is not in the gRPC proto.

## Context provider contract (H3-1, ADR 0002)

- `ContextRequest` and `ContextItem` are data only, in `tekmor-core`. The provider trait and the first providers come in later PRs.

## Context gather (H3-1b, ADR 0002)

- `tekmor/src/context.rs` gathers file and lesson context into one list of typed items, then splits it back for the route response. Its owner is the core-crate feature map.

## Evidence source (H3-2, ADR 0002)

- Receipts read `EvidenceRecord` from `tekmor-core`, converted from store rows at the store boundary. Bench proofs are not a source until bench records a code hash and head SHA.

## Gate rule socket (H3-3, ADR 0002)

- Bash, commit and route gates return `GateVerdict` through `evaluate` in `tekmor-core`. A block from any gate beats a warning. The protected-changes check stays a receipt report.
