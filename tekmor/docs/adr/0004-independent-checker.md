# ADR 0004: Independent checker and the CI receipt (H3-6, H4-1, H4-2)

Status: Accepted by the owner's direction to complete H3 and H4 with the optimal design. Supersedes the open question in ADR 0002 about spawning agents from a host hook.

## Context

H3-6 asked for an independent checker: a verdict on a change that the agent doing the work does not produce. The plan proposed spawning a checker agent from a host hook. That needs its own design because it runs model calls from hooks, adds latency to every tool call, and makes the verdict depend on another model's judgment.

H4-1 and H4-2 ask for a receipt on each pull request and a CI check that fails without a passing receipt. The plan's open problem was how CI can trust a receipt the author produced locally.

## Decision

1. No agent is spawned from a hook. The independent checker is the receipt, `tekmor verify receipt`. Its verdict comes only from recorded runs for the current head and code tree. It exits 1 unless the verdict is Passed.
2. CI produces its own evidence. The `receipt` workflow (`.github/workflows/receipt.yml`) builds `tekmor` in a fresh `TEKMOR_HOME`, runs `tekmor verify -- .verify/run.sh`, then checks the receipt. A local receipt is never trusted by CI. No signing key is needed.
3. The receipt is written to the job summary (`$GITHUB_STEP_SUMMARY`). It is not posted as a pull request comment. That needs no write permission and creates no outward post.
4. The workflow has `contents: read` only.

## Rejected

- Spawning a checker agent from a hook: model calls on the tool path, latency on every call, and a verdict that is a second opinion, not evidence.
- Signed receipts: the key would live in CI, and a leaked key forges receipts. Re-running the allowlist in CI gives the same guarantee with no key.
- Posting the receipt as a PR comment: outward-facing and needs `pull-requests: write`. The job summary shows the same content.

## Verified

- In an empty `TEKMOR_HOME`, `tekmor verify -- cargo check` followed by `tekmor verify receipt --project .` printed `Verify receipt: PASSED` and exited 0. This is the path the workflow runs, using a quick allowlisted command in place of `.verify/run.sh`.
- The full `.verify/run.sh` run in CI is not verified here. Its tools (for example `hurl`) may be missing on the hosted runner. A failure there is the check doing its job, but it has not been observed.

## Owner action required

- Add the "Verified receipt for this change" check to the default-branch ruleset. Until then the workflow runs and reports but does not block merges. This is a repository setting the agent cannot change.

## Consequences

- The receipt is the single verdict used by hooks (`stop_gate`), the local CLI, and CI.
- A change whose allowlist run fails in CI cannot merge once the check is required.
