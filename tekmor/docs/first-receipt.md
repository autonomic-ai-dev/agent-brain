# Your first verify receipt in about five minutes

A receipt answers one question: did the verify command pass for the commit and code state I have right now? You can paste the answer into a pull request.

The steps below match the regression test in `tests/first_receipt.rs`. That test creates an empty tekmor home and a small git project, then runs every step in order.

## 1. Install

On macOS with Apple Silicon:

```bash
brew tap autonomic-ai-dev/tap
brew install tekmor
```

## 2. Connect your coding host

```bash
tekmor install --global --claude-code   # or --cursor, --codex, --gemini, --opencode, --antigravity, --vscode
tekmor doctor
```

Run `tekmor install --all` to configure every supported host. See [the support matrix](support-matrix.md) for what each host gets.

## 3. Initialise verification in your project

From the root of a git repository with at least one commit:

```bash
tekmor verify init
```

This creates `.verify/verify.toml`. Its `commands` list is the allowlist. The default list depends on the files it finds: `cargo test` for a Cargo project, `npm test` for a Node project, and `make test` when a `Makefile` exists.

## 4. Run the verify command

```bash
tekmor verify -- make test     # or: cargo test, npm test, pytest
```

Only commands that match the allowlist run. tekmor runs the command itself and records its exit code. It does not trust a result that the agent reports.

## 5. Read the receipt

```bash
tekmor verify receipt
```

Example output (your values will differ):

```
Verify receipt: PASSED
  command:   make test
  exit code: 0
  recorded:  2026-11-08T10:12:44Z
  source:    cli
  artifact:  .verify/artifacts/3f9c2a1e-8b7d-4c6e-9a10-5d2b7e8f1a20.log
  commit:    1a2b3c4d5e6f
  code hash: 9f8e7d6c5b4a
```

The `artifact` line is the full output of the run, saved in the project. It appears when the project has a `.verify/` folder. Open it to see the exact output behind the verdict.

For a pull request, paste the markdown version:

```bash
tekmor verify receipt --markdown
```

## What the verdict means

| Verdict | Exit code | Meaning |
|---|---|---|
| `PASSED` | 0 | Your verify command ran for this commit and code state and exited 0. |
| `FAILED` | 1 | The newest run for this commit and code state exited non-zero. The receipt shows that exit code. |
| `NOT VERIFIED` | 1 | No run exists for this commit and code state. Run the verify command again. |

## Rules that catch stale evidence

- **A new commit needs a new run.** A receipt is tied to the commit that was checked out when the run happened.
- **Editing code needs a new run.** The code hash covers uncommitted changes to files under `src/`, `crates/`, `tests/`, `hooks/`, and `Cargo.*`. Editing a README does not invalidate a receipt.
- **Only commands run by `verify --` count.** Results that a hook or an MCP call reports are not receipts.
- **Runs recorded before commit tracking existed do not count.** Run the command again after upgrading.

## What a receipt does not prove

A receipt proves that the command you recorded passed. It does not prove that the command covers your change. If `make test` runs three unit tests and your change breaks a fourth area, the receipt still says `PASSED`. Choose a command that covers the area you changed.

## Clean-machine check

The automated test covers the sequence above on a fresh tekmor home and a fresh git repository. It does not cover a clean macOS install, Homebrew itself, or host configuration. Those steps still need a manual run on a new machine.
