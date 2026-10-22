---
name: forensics
description: "Diagnose stack traces, triage production logs, analyze runtime crashes, and build minimal reproductions. Settle defect causes with runtime proof before touching production code."
disable-model-invocation: true
---

# Forensics: Stack Trace Analysis & Bug Reproduction

A bug that cannot be reproduced is a bug that is not understood. `forensics` provides the rigorous procedure to analyze stack traces, parse logs, and construct failing reproductions.

## The Forensics Protocol

### 1. Stack Trace Deconstruction
- Parse the stack trace from bottom to top to identify:
  - The exact failure point (file, line number, column, exception class).
  - The last user-code frame before entering library or runtime code.
  - The entrypoint frame (HTTP request handler, background job runner, CLI command).
- Cross-reference line numbers with the repository commit history to ensure the trace matches the current working tree.

### 2. Log & State Triage
- Correlate timestamps across logs to identify cascading failures.
- Inspect inputs immediately preceding the crash: payload size, null/undefined properties, type mismatches, or unexpected characters.
- Identify concurrency markers: race conditions, deadlock signals, connection pool exhaustion, or timed-out lock acquisitions.

### 3. Build the Reproduction (Mandatory Gate)
- Construct the smallest standalone test case, unit test, or Hurl request that reliably reproduces the exact error output and exit code.
- Run the reproduction and capture the failing output in `.verify/artifacts/reproduction.log`.
- **Hard Rule:** Do not touch production code until you observe the reproduction failing with your own eyes.

### 4. Root Cause Classification
- Distinguish between:
  - **Symptom:** The crash point (e.g., `NullPointerException`, `IndexOutOfBounds`).
  - **Root Cause:** The flawed invariant or missing boundary check that permitted invalid state to exist in the first place (`principle-fix-root-causes`).
- Fix the root cause; never apply band-aids to the crash point.
