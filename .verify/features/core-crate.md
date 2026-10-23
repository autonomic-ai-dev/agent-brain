# core-crate

## Sub-features

- `core-routing-contract`: Behavior contract rules that every routed response carries.
  - Intent: Keep the rules agents must follow on every route defined in one pure place.
  - Behaviour: Returns the five must-apply rules; the search-before-asking and evidence rules are present in their text.
  - Outcome: `behavior_contract` tests pass.
- `core-retrieval-ranking`: Reciprocal rank fusion and cross-encoder reranking of candidate items.
  - Intent: Let an exact symbol match outrank weaker lexical matches, and keep a strong hybrid leader on top.
  - Behaviour: RRF favors items found by more than one source; an exact identifier match boosts the candidate; rerank keeps the leader.
  - Outcome: `retrieval_fusion` and `cross_encoder_rerank` tests pass.
- `core-query-decomposition`: Splits a compound query into sub-queries without breaking phrases.
  - Intent: Search each part of "X and Y" separately while keeping quoted phrases whole.
  - Behaviour: Splits on conjunctions such as "and also"; keeps natural phrases intact.
  - Outcome: `query_decompose` tests pass.
- `core-memory-decay`: Time-based decay of memory weight, slowed by reinforcement.
  - Intent: Let unused memories fade and useful ones persist.
  - Behaviour: A fresh memory keeps full weight; an old one without reinforcement decays; a higher useful count slows decay.
  - Outcome: `memory_decay` tests pass.
- `core-boundary`: The core crate depends on no storage, runtime, or IO crate.
  - Intent: Keep routing logic pure and testable without a database, daemon, or network.
  - Behaviour: The normal dependency tree of `agent-brain-core` is checked against a denylist of IO and runtime crate names.
  - Outcome: `.verify/checks/core_boundary.sh` exits 0 and prints the holding message.

## How to get to it (user POV)

- Run `agent-brain route_task` from any repository. The must-apply rules it returns come from `behavior_contract` in the core crate.
- Ranking and decomposition happen inside `route_task` when a query is served; there is no separate user command.
- Developers reach the crate at `agent-brain/crates/agent-brain-core`. The `agent_brain::types` and other re-exported paths still resolve for existing callers.

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test -p agent-brain-core`
- **Static Check:** `.verify/checks/core_boundary.sh`
- **Allowlisted Verification:** `.verify/run.sh` (runs the static check and the unit tests in its core stage)
- **Preconditions:** Rust toolchain and the repository's locked dependencies. No service, daemon, or database is needed.

### Verification Steps

1. **core-unit-tests** (sub-features: core-routing-contract, core-retrieval-ranking, core-query-decomposition, core-memory-decay)
   - Request: `cargo test -p agent-brain-core`
   - Why: Proves each pure ranking, decay, decomposition, and contract behavior with its own test.
   - Expected: 11 passed, 0 failed.
   - Impact: A failure means routing logic changed behavior. Fix the cause; do not change the expectations.
2. **core-boundary-check** (sub-feature: core-boundary)
   - Request: `.verify/checks/core_boundary.sh`
   - Why: Proves the core crate pulled in no storage, runtime, or IO dependency.
   - Expected: Exit 0 with "core boundary holds".
   - Impact: A failure means a storage or IO crate entered the core. Move that code back to the adapter crate.

## Gotchas

- The boundary check is a denylist of crate names. A new IO crate that is not on the list will pass the check. Extend the list in `.verify/checks/core_boundary.sh` when a new IO dependency is added.
- `commit_gate` reads stdin, so it stays in the adapter crate. It is not covered by the core boundary.
- The re-exports in `src/lib.rs` keep the old `agent_brain::<module>` paths valid. Removing a re-export is a breaking change for any caller that uses the old path.
- Earlier work on this branch is uncommitted. The split and its checks are not yet committed on their own.
