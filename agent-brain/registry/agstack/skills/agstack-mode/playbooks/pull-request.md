# Playbook: Pull Request & Delivery

Use this playbook whenever you are preparing, chunking, and opening a ready pull request for review.

---

## Process Steps

### Step 1: Working Tree Audit & Diff Hygiene
- Inspect git status and full diff against `master`:
  ```bash
  git status
  git diff master
  ```
- Verify zero unintentional changes, no stray debug logs, no commented-out code, and no newly introduced narration comments (`scripts/check-added-comments.sh`).
- Confirm all touched files are necessary. Squeeze out unnecessary lines (`skills/token-efficiency/`).

### Step 2: Atomic Chunked Commits
- Group related changes into logical atomic commits rather than one massive commit.
- Use Conventional Commits (`feat(scope): ...`, `test(scope): ...`, etc.).
- Ensure every commit builds cleanly and passes focused tests.

### Step 3: Run Full Allowlisted Quality Gates
- Execute the complete verification harness:
  ```bash
  sh .verify/run.sh all
  ```
- Capture exit codes, logs, and coverage reports into `.verify/artifacts/<RUN_ID>/`.

### Step 4: Author Rich Pull Request Description
- Author the PR description following `skills/pull-request/SKILL.md`:
  - **Summary & Problem Intent:** What was built and why.
  - **High-Level Design (HLD):** Architecture diagram (Mermaid chart).
  - **Low-Level Design (LLD):** Data contracts, interface signatures, error models, and rollback plan.
  - **Commit Log:** Atomic chunk progression.
  - **Blast Radius:** Callers verified, downstream impact.
  - **Security & Injection Defense:** Zero secrets committed, dependency audit clean, delimiter boundaries and prompt injection defenses verified.
  - **Evidence Record:** Table with commands, exit codes, and coverage metrics.

### Step 5: Final Sanity Check
- Verify the PR branch is rebased cleanly onto the latest `master`.
- Present the final PR title and body to the user or push to origin.
