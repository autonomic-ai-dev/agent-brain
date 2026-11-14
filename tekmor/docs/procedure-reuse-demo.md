# Procedure reuse demo

This demo shows tekmor reusing what worked in one task when a second, similar task starts. The same steps are run by `tests/procedure_reuse.rs`, so the demo is checked on every test run.

## What you need

- A git repository with at least one commit and a test command. This demo uses a `Makefile` whose `test` target passes when a file named `.pass` exists.
- tekmor installed, with `tekmor verify init` already run in the repository.

## Task one: a test fails, then passes

```bash
tekmor verify -- make test     # fails: .pass does not exist yet
touch .pass                          # the fix
tekmor verify -- make test     # passes
```

Each `verify --` run is recorded as a trajectory for the procedure `make`. The record keeps the outcome and the command, so a later task can see which attempt worked.

## Task two: the same kind of work

A new task starts, and the agent asks for procedures that match its request. Through MCP, the tool is `recall_procedures`. The MCP tools accept calls only after `route_task` has succeeded in the session:

```json
{ "query": "make test", "task_kind": "verification", "limit": 5 }
```

The result for `make`, with the values the test checks:

| Field | Value | Meaning |
|---|---|---|
| `node_id` | `make` | The procedure learned from task one |
| `total_runs` | `2` | Both attempts are counted |
| `success_rate` | `0.5` | One of the two attempts passed |
| `task_kind` | `verification` | Matches the task kind of the earlier runs |

The agent now knows the procedure, the success rate, and the command that worked, so it can try `make test` first instead of rediscovering the fix.

## What the demo does not claim

- The procedure is only as useful as the runs behind it. Two attempts are a small sample, and the success rate reflects that.
- An unrelated request does not recall this procedure in the regression test (`an_unrelated_task_does_not_recall_the_procedure`). The check is a score threshold, not a guarantee for every query.
- Procedures are learned on this machine. Nothing is shared with other users.

## Run the demo as a test

```bash
cargo test -p tekmor --all-features --test procedure_reuse
```
