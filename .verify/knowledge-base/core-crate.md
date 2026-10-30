---
feature: "[[../features/core-crate]]"
---

# Core crate

## What it does

`agent-brain-core` holds the pure routing logic of agent-brain: the behavior-contract rules, retrieval ranking, query decomposition, and memory decay. It has no storage, runtime, or IO dependency, so the logic can be tested with no database, daemon, or network.

## How it works

1. **Behavior contract.** `behavior_contract` returns the rules every routed response carries. The must-apply rules and the search-before-asking and evidence rules are in its text.
2. **Retrieval ranking.** Reciprocal rank fusion (`retrieval_fusion`) favors items found by more than one source. An exact identifier match boosts a candidate. Cross-encoder reranking (`cross_encoder_rerank`) keeps a strong hybrid leader on top.
3. **Query decomposition.** `query_decompose` splits a compound query such as "X and also Y" into sub-queries, and keeps quoted phrases whole.
4. **Memory decay.** `memory_decay` lowers the weight of old, unreinforced memories. A higher useful count slows the decay. A fresh memory keeps full weight.
5. **Boundary.** `.verify/checks/core_boundary.sh` reads the normal dependency tree of the core crate and fails if it contains a storage, runtime, or IO crate.

## Where the code is

- Crate: `agent-brain/crates/agent-brain-core`
- Re-exports in `src/lib.rs` keep the old `agent_brain::<module>` paths valid
- Boundary check: `agent-brain/.verify/checks/core_boundary.sh`

## How to use it

- There is no separate user command. Running `agent-brain route_task` from any repository uses this logic.
- Developers work on it through `cargo test -p agent-brain-core`.

## How it is verified

- `cargo test -p agent-brain-core`: 11 tests, all passing
- `sh .verify/checks/core_boundary.sh`: exits 0 with "core boundary holds"

## Known limits

- The boundary check is a denylist of crate names. A new IO crate that is not on the list passes. Extend the list when a new IO dependency is added.
- `commit_gate` reads stdin, so it stays in the adapter crate and is outside the core boundary.
- Removing a re-export in `lib.rs` breaks any caller that uses the old path.

## Related

- Feature map: [[../features/core-crate]]
- Routing and verify CLI: [cli-verify-and-memory](cli-verify-and-memory.md)
