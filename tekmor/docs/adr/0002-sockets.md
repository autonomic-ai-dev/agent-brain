# ADR 0002: Sockets for providers, evidence, gate rules and readers (H3)

Status: Proposed. No code until the owner approves this note. Depends on ADR 0001 for the gate-rule socket.

## Context

The plan's Horizon 3 goal: a new feature plugs in without touching the core, and stays on by default only if it shows a number.

Today, each feature is wired into the engine, the MCP server or the host hooks by hand. The core crate, `tekmor-core`, holds the routing types and scoring logic. Its boundary check (`.verify/checks/core_boundary.sh`) forbids storage, runtime and IO crates in it.

Two conditions block implementation today:

- The socket contracts should come from two real implementations of the same socket. The plan names the context provider, using H2-3 (lessons) and H2-4 (file context).
- Each move must show no behaviour change. The plan proves that with the retrieval eval from H2-6. That eval has not passed yet: it reports 29 of 58 cases, against a threshold of 0.85. Without a passing eval, a "no change" claim is only a claim.

## Decision

Define the four sockets as pure data types and functions in `tekmor-core`. Implement them in the `tekmor` binary. Do not add runtime plugin loading. Providers are modules compiled into the binary.

The four sockets:

1. Context provider. Input: the route request subset (paths, task kind, project). Output: typed items, each with a token cost and a "why now" reason. The engine merges providers under the token budget.
2. Evidence source. A row carries project, code hash, head sha, command or check id, result, artifact path, and the requirement ids it proves (from H1-3). `tekmor verify` is the first source.
3. Gate rule. Input: the event and project context. Output: a verdict with a reason and an alternative. The Bash policy, the commit gate, the protected-check rule and the route gate from ADR 0001 all use this one signature.
4. Reader. A query layer shared by the receipt, `status`, `doctor`, `stats` and the dashboard.

## Migration order

Each step is its own PR with no behaviour change, proven by the eval once it passes.

1. H3-1: context provider. Move feature-map file context, lessons, memory, then skill and rule retrieval.
2. H3-2: evidence source. Move `verify` into the first source. Add the bench proofs as the second source.
3. H3-3: gate rule. Requires ADR 0001 to land first.
4. H3-4: reader. Move the receipt, `status`, `doctor`, `stats` and the dashboard onto one query layer.
5. H3-5: extension delivery. Fix the `ext add` error message and decide whether `get_local_graph` ships in the release binary.
6. H3-6: independent checker. Needs its own ADR, because it spawns agents from a host hook.

## Precondition

Step 1 cannot start until the retrieval eval passes at its threshold, or until a baseline snapshot of current retrieval output is recorded. Without one of the two, the "no behaviour change" claim has nothing to compare against.

## Rollback

Each step is one PR. Reverting it restores the previous wiring. No schema change is planned in H3. Any socket that adds a setting ships with the setting that turns it off.

## Risks

- Two implementations may not be enough to shape the contract. The first socket may need a second revision.
- Moving retrieval changes ranking. The eval is the only proof, and it is currently failing.
- The core boundary check must still pass. Any socket that needs IO stays in the binary.

## Amendment 1: typed context items (H3-1b)

The first draft gave `ContextItem` an id, a kind, a token cost and a "why now" reason. The file-context and lesson providers return different shapes, and neither fits those fields. Neither can be rebuilt from a generic item without losing data the route response needs.

`ContextItem` now carries a typed payload: `File(FileContext)` or `Lesson(LessonRec)`, both defined in `tekmor-core`. The route response keeps its current fields, so clients see no change.

Token cost and "why now" are dropped for now. They come back with the budget merger, which is the first thing that needs them. The provider trait is still deferred. Each provider is a plain function, and one gather step builds the items.

## Amendment 2: evidence source (H3-2)

The evidence socket is `EvidenceRecord` in `tekmor-core`. It holds the fields the receipt reads, and `is_run_for` decides whether a record proves a given head and code tree. The store row converts into it at the store boundary, so the receipt no longer depends on storage types.

The bench proofs are not a second source yet. The bench baseline files under `.verify/baseline` record no code hash or head SHA, so they cannot satisfy `is_run_for`. A second source waits until bench records both. Adding a source then means adding a row mapping, with no change to the receipt.

## Amendment 3: gate rule socket (H3-3)

Gate rules implement `GateRule` in `tekmor-core` (`crates/tekmor-core/src/gate_rule.rs`). Each returns `Allow`, `Warn` or `Block`. `evaluate` returns the first block, otherwise the first warning, otherwise allow. Three gates use it: the command policy (`CommandRule`), the commit gate (`CommitRule`), and the route gate (`RouteRule`). The route rule reads its state at construction, so the input stays tool and command only.
Not converted: the protected-changes check (`protected_changes` in `tekmor/src/receipt.rs`). It lists changed protected files in the receipt and never returns a block verdict, so it is a report, not a gate.

## Amendment 4: evidence reader (H3-4)

Every evidence query goes through `tekmor/src/evidence_reader.rs` (`recent_runs`, `project_runs`), which returns `EvidenceRecord`. Callers: receipt, stop gate, verify and suggest. `EvidenceRecord.source` stays optional, because a `NULL` source is not counted by the stop gate but is counted as cli by the receipt. Keeping the column faithful to the row keeps both behaviours. The dashboard, doctor and stats commands do not read evidence rows, and status reads only the route state file, so they have no evidence query to move.
Known inconsistency, not changed: the stop gate and the receipt disagree on rows with a `NULL` source. The stop gate does not count them; the receipt does. Resolving it changes gate behaviour, so it needs its own decision.
