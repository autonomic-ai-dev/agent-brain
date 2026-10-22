# Playbook: Autonomous Run

Use this playbook whenever you are driving an end-to-end task, long-running batch implementation, or overnight milestone run.

---

## Process Steps

### Step 1: Sequential Milestone Decomposition
- Break the high-level objective into sequential, verifiable milestones.
- Each milestone must have an explicit verification command that returns 0 on success.
- Record the ordered milestone checklist.

### Step 2: Incremental Execution & Gating
- Execute one milestone at a time.
- After each milestone, run the corresponding verification check:
  ```bash
  sh .verify/run.sh <feature>
  ```
- **Hard Gate Rule:** If any check or gate fails, STOP immediately. Do NOT proceed to the next milestone hoping it will resolve. Fix the failure before continuing.

### Step 3: Self-Contained Evidence Trail
- Log every executed command, exit code, and test report to `.verify/artifacts/<RUN_ID>/`.
- Preserve proof across cleanups. Teardown kills spawned processes by recorded PID; it NEVER deletes `.verify/artifacts/`.

### Step 4: Final Evidence Record
- Compile the final summary in `evidence.md`:
  - Milestones attempted and completed.
  - Test commands executed with exit codes.
  - Measured line and branch coverage.
  - Residual risks, operational gotchas, or unconfirmed expectations.
