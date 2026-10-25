# core-crate knowledge base

## Owned code

- `agent-brain/crates/agent-brain-core`: the pure routing core. This change does not modify it.

## Behaviour that matters

- The core depends on no storage, runtime, or IO crate. `.verify/checks/core_boundary.sh` enforces this with a denylist.
- The `brain_git_*` and `grep_search` changes in this work live in the adapter crate (`agent-brain/src`), not the core. Their owner is `cli-verify-and-memory`.

## Verification

- Not re-run for this change. The core crate is untouched, so `cargo test -p agent-brain-core` and the boundary check were not required by it. The `agent-brain verify -- cargo test --all-features` run covers the whole crate, including core.
- Known gap: the boundary check is a denylist. A new IO crate not on the list passes the check.
