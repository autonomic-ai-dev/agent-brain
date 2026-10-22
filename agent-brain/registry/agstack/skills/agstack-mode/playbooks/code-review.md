# Playbook: Code Review & Interrogation

Use this playbook before declaring any task complete, opening a pull request, or committing changes.

---

## Process Steps

### Step 1: Full Working Tree & Diff Audit
- Inspect `git diff` against `master`:
  - Check that only intended files were modified.
  - Verify no accidental edits, temporary debug logs, or commented-out corpses exist.
  - Run `scripts/check-added-comments.sh` to guarantee no AI narration comments were introduced.

### Step 2: Adversarial Security, Concurrency & Lean Audit
- Ground in `security-audit`, `jailbreak-defense`, `interrogate`, and `token-efficiency`:
  - **Secrets:** Verify no tokens, passwords, keys, or .env values were touched or added.
  - **Jailbreak & Prompt Injection Defense:** Are external or user inputs segregated with delimiters (`<untrusted_input>`)? Are delimiter escape collisions handled?
  - **Injection & Boundaries:** Are inputs validated and parameterized against SQL/command injection? Are file paths canonicalized against traversal?
  - **Excessive Agency:** Are destructive tool operations gated with approval?
  - **Concurrency & Timeouts:** Do external calls have explicit timeouts? Are retries bounded with backoff and jitter?
  - **Resource Cleanup:** Do error paths release locks, file handles, and database connections?
  - **Lean check:** Is every added line needed? Can we delete dead code or reuse an existing helper?

### Step 3: Blast Radius & Regression Verification
- Run `blast-radius` checks on modified symbols:
  - Did any caller signature break?
  - Does every fixed defect have an accompanying regression test that failed on the unfixed baseline?
  - Did any existing test break, get skipped, or get loosened? (Zero tolerance for test weakening).

### Step 4: Quality Gate Execution & Review Report
- Execute the full repository verification runner:
  ```bash
  sh .verify/run.sh all
  ```
- Format review findings using the four-part structure:
  - Group into `Must Fix`, `Should Fix`, and `Nice to Have`.
  - For each finding: `What this is`, `Problem`, `Fix`, `If we skip it`.
- Output final verdict:
  - `Verdict: Ship.` or `Verdict: Fix X and Y first.`
  - `Lean: -<N> lines possible.`
  - Complete evidence record with exit codes and measured coverage.
