# Playbook: Bug Fix

Use this playbook whenever you are fixing a defect, crash, regression, or unexpected behavior.

---

## Process Steps

### Step 1: Reproduce with Runtime Evidence (Mandatory)
- Do NOT jump straight to patching code.
- Write a minimal failing test case or reproduction script that triggers the exact failure.
- Run the reproduction and observe the failing exit code and concrete error output.
- If you cannot reproduce the bug, you do not understand it. Trace code further before touching anything.

### Step 2: Root Cause Analysis (Attack the Premise)
- Trace the defect to its fundamental root cause.
- Do NOT paper over symptoms with defensive `if (x == null)` checks or workaround patches.
- Ask: What flawed assumption or broken contract allowed this state to occur?

### Step 3: Implement Minimal, Focused Fix
- Ground the fix in `principle-fix-root-causes` and `principle-immutability-first`.
- Make the smallest change that completely resolves the root cause.
- Avoid incidental changes, reformatting unrelated code, or adding speculative features.

### Step 4: Regression Test Proof
- Verify that your test fails on the unfixed baseline and passes with the fix.
- Ensure the regression test verifies observable behavior and meaningful failure modes, not implementation trivia.

### Step 5: Verification Suite Run
- Run the feature's verification suite through the allowlist:
  ```bash
  sh .verify/run.sh <feature>
  ```
- Verify that both unit tests and functional Hurl contracts pass cleanly.

### Step 6: Code Hygiene & Evidence Record
- Run `no-comments` to strip any temporary debug code, print statements, or narration comments.
- Run `unslop` to ensure the patch is clean, idiomatic, and concise.
- Output the final report with the reproduction command, root cause summary, and test verification output.
