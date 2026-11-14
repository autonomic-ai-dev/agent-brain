---
feature: core-crate
knowledge_base: "[[../knowledge-base/core-crate]]"
related:
  - "[[cli-verify-and-memory]]"
---

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
  - Behaviour: The normal dependency tree of `tekmor-core` is checked against a denylist of IO and runtime crate names.
  - Outcome: `.verify/checks/core_boundary.sh` exits 0 and prints the holding message.

- `evidence-reader`: One query layer for stored verify runs (H3-4, ADR 0002 amendment 4).
  - Behaviour: `tekmor/src/evidence_reader.rs` (`recent_runs`, `project_runs`) returns `EvidenceRecord`s for receipt, stop gate, verify and suggest. `source` stays optional so each caller keeps its NULL-source behaviour.
  - Outcome: Evidence queries live in one module. The stop gate and receipt still disagree on NULL-source rows (ADR 0002 amendment 4).
- `gate-rule-socket`: One rule shape for the Bash, commit and route gates (H3-3, ADR 0002 amendment 3).
  - Behaviour: `GateRule` and `evaluate` in `crates/tekmor-core/src/gate_rule.rs` return the first block, else the first warning, else allow. `CommandRule`, `CommitRule` and `RouteRule` adapt the three gates in the binary.
  - Outcome: The bash gate, commit gate and route CLI output is unchanged. A new gate adds one adapter.
- `evidence-record`: The evidence a receipt reads, and when a run proves a head and code tree (H3-2, ADR 0002 amendment 2).
  - Behaviour: `EvidenceRecord::is_run_for` in `crates/tekmor-core/src/evidence.rs` accepts only cli and orchestrator runs for the same head and code hash. The store converts rows into it.
  - Outcome: The receipt is unchanged for users. Bench proofs are not a source yet.
- `route-file-context-types`: `FileContext` and `LessonRec` are optional fields of `RouteTaskResponse`.
  - Behaviour: `crates/tekmor-core/src/types.rs` defines both structs. `file_context` and `lessons` are omitted from the serialized response when empty.
  - Outcome: Clients that ignore the fields see no change.

- `context-provider-contract`: The typed items a context provider returns (H3-1, ADR 0002 amendment 1).
  - Behaviour: `ContextItem` in `crates/tekmor-core/src/context_provider.rs` carries a `File` or `Lesson` payload. The binary gathers both in `tekmor/src/context.rs`, and route_task splits them back into its two response fields.
  - Outcome: The route response is unchanged. New providers add a payload variant.

## Code

- `tekmor/crates/tekmor-core`

## How to get to it (user POV)

- Run `tekmor route_task` from any repository. The must-apply rules it returns come from `behavior_contract` in the core crate.
- Ranking and decomposition happen inside `route_task` when a query is served; there is no separate user command.
- Developers reach the crate at `tekmor/crates/tekmor-core`. The `tekmor::types` and other re-exported paths still resolve for existing callers.

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test -p tekmor-core`
- **Static Check:** `.verify/checks/core_boundary.sh`
- **Allowlisted Verification:** `.verify/run.sh` (runs the static check and the unit tests in its core stage)
- **Preconditions:** Rust toolchain and the repository's locked dependencies. No service, daemon, or database is needed.

### Verification Steps

1. **core-unit-tests** (sub-features: core-routing-contract, core-retrieval-ranking, core-query-decomposition, core-memory-decay)
   - Request: `cargo test -p tekmor-core`
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
- The re-exports in `src/lib.rs` keep the old `tekmor::<module>` paths valid. Removing a re-export is a breaking change for any caller that uses the old path.
- Version: 1.0.0-rc5, set in `tekmor/crates/tekmor-core/Cargo.toml`. See CHANGELOG.md for the release contents.
- `RouteTaskResponse.response_tokens` (2026-11-13) is optional and omitted when unset. The MCP handler fills it with the size of the whole serialized response; the engine leaves it `None`.
