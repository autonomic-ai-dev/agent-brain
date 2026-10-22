---
name: principle-fix-root-causes
description: "Resolve the fundamental defect rather than adding defensive workaround patches or symptom suppressions. Attack the premise."
disable-model-invocation: true
---

# Principle: Fix Root Causes

## Invariant
Never patch around a symptom. If a bug is caused by a race condition, corrupted state, or incorrect assumption, fix the condition at its source rather than adding another defensive check downstream.

## Application
1. **Attack the Premise:**
   - If multiple fixes for a bug fail, write down the shared underlying premise and verify it directly. The premise is often wrong.
2. **Eliminate Workaround Sermons:**
   - Never write comments like `// hack to prevent crash` or add special casing for an invalid state that should never have been created. Make the invalid state impossible at creation time (`principle-foundational-thinking`).
3. **Regression Tests:**
   - Every fixed bug must have a dedicated test that reproduces the failure on the unfixed code and passes cleanly on the fix.
