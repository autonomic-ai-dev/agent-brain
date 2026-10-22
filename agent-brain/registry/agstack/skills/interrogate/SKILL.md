---
name: interrogate
description: "Adversarial pre-completion review across failure modes, concurrency, error paths, and code quality. Actively attempts to break the diff before reporting completion."
disable-model-invocation: true
---

# Interrogate: Adversarial Review

Before declaring any change complete or opening a pull request, run an adversarial self-review. Do not review with the optimism of the author; review with the skepticism of a production incident responder.

## The Review Angles

### 1. The Concurrency & Failure Angle
- What happens if the network connection drops mid-operation? Is partial state leaked or recoverable?
- Are timeouts explicitly set on every external call?
- Are retries bounded with backoff and jitter, and restricted strictly to idempotent operations?
- Does every error path release locks, sockets, and temporary files?

### 2. The Edge & Boundary Angle
- What happens on empty inputs, zero values, maximum integers, and oversized payloads?
- Is there any implicit calendar dependency (e.g., month rollover, leap year, timezone shift)?
- If reference data changes live, will the system crash or handle the divergence gracefully?

### 3. The Security & Access Angle
- Are inputs sanitized and parameterized (no raw SQL/shell injection)?
- Are credentials, tokens, or PII logged or exposed in error messages?
- Are authorization checks evaluated before executing business logic?

### 4. The Verification & Quality Angle
- Did every fixed defect receive a regression test that failed on the unfixed baseline?
- Is there any test that was commented out, weakened, or bypassed?
- Are any new failure modes unobservable in metrics or logs?

## Review Findings Structure

Format all review findings into three clear groups:
1. **Must Fix:** Crashes, security holes, data loss, broken callers outside the diff, race conditions under load.
2. **Should Fix:** Risky logic without a regression test, significant performance regressions, bloated boilerplate that should not exist.
3. **Nice to Have:** Minor speed-ups, shorter idiomatic formulations.

Every individual finding must contain all four parts:
```markdown
1. **[Finding Title]** (`path/to/file.ts:L40-L55`)
   - **What this is:** What the code does in plain words.
   - **Problem:** The concrete case or input that causes failure.
   - **Fix:** The smallest fix that solves it (prefer deleting code).
   - **If we skip it:** The observable failure that will occur in production.
```

End the review with an unambiguous verdict:
- `Verdict: Ship.` OR `Verdict: Fix 1 and 3 first.`
- `Lean: -<N> lines possible.` (when bloated code can be removed)
- `Not checked:` one line stating any boundary that could not be verified.
