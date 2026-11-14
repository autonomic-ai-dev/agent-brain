# ADR 0003: The ledger across people (H4)

Status: Blocked. Do not start. This note records the gate and the open design problems. It is not a plan to build.

## Gate

The plan's rule: start only after single-user retention is shown. The evidence needed is the review week (`tekmor/docs/delegation-plan.md`, section 12) on this repo and two others, with the Horizon 1 exit gate numbers: gate blocks, hand-backs, bash warnings, routes per session, and tasks sent back after "done".

Status of the gate:

- The review week has not run on other repositories.
- No retention numbers exist outside the owner's machine.
- The product validation (see the validate-idea verdict: "needs more validation") found no paying or outside users.

So the gate is not met. H4 does not start.

## What H4 would be

From the plan:

- H4-1: a receipt on each pull request. `tekmor verify receipt --markdown` already exists. A documented step posts it with `gh pr comment --body-file`. Posting is outward-facing, so the command runs only when the user or CI runs it, never from the stop gate.
- H4-2: a CI check that fails when the head commit has no PASSED receipt. Receipts are stored in the local ledger, so CI cannot read them as they are.
- H4-3: shared lessons through the sync bundle (`tekmor export` and `import`, in `tekmor/src/sync`). Lesson excerpts pass the secret check again on export.
- H4-4: a team view of which checks catch the most failures. This is a reader on the H3 query layer.

## Open design problems

- H4-2 needs an artifact protocol. The check has to receive a receipt that CI can verify without trusting a file the author wrote. Options to decide: sign the receipt with a key held in CI, or re-run the check in CI and compare the code hash. The second needs no key, but it re-runs the verify step.
- Lessons carry command output excerpts. An excerpt can leak a path or a name the team does not want shared. The export needs a review step, not only the secret check.
- A team view implies a shared store. The sync bundle is per-user today. Choosing a shared store is a consequential decision that needs its own ADR.

## Decision

None yet. Revisit after the review week. If retention holds, write the H4-2 receipt protocol first, because the other steps depend on it.

## Rollback

Nothing is built, so nothing to roll back.

## Amendment 1: decisions taken (H4, owner direction)

- H4-1 and H4-2 are implemented in ADR 0004: the CI workflow records its own run and checks the receipt, with the receipt in the job summary.
- H4-4: a local per-check summary (`tekmor verify summary`) shows which checks fail most. A team view over several people needs a shared store, which is a separate decision and needs its own ADR. Not built.
- H4-3 (lessons in the sync bundle): see the H4-3 PR for the export rule and secret check.

## Amendment 2: lessons in the sync bundle (H4-3)

- Lessons leave a machine only with `tekmor export --with-lessons`. The default bundle is unchanged. Owner decision: opt-in, because lesson excerpts are command output.
- Each lesson is checked with the secret rule before it is written, and again on import. A lesson that names a secret, a `.env` or a `.pem` is left out, and counted as skipped on import.
- `lessons.jsonl` is covered by a checksum in the manifest. A bundle that has the file but no checksum is refused.
- Not verified end to end: importing a bundle that contains a lesson. The unit tests cover the filter and the parse. The store insert is the existing `insert_lesson`.
