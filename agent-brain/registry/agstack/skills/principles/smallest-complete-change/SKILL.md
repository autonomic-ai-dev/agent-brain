---
name: principle-smallest-complete-change
description: "Solve the entire problem with the smallest working diff. Rethink whether code is needed, reuse existing patterns, and minimize touchpoints without cutting validation, security, or tests."
disable-model-invocation: true
---

# Principle: Smallest Complete Change

## Invariant
The shortest working diff that completely solves the task and handles edge cases wins. Before adding code, rethink whether that code is really needed.

## Application
1. **Reuse Over Creation:** Check the repository, standard library, and existing dependencies before writing new utilities.
2. **Small Blast Radius:** Confine edits to the minimum set of files and functions necessary. Keep interface contracts intact.
3. **Finish the Entire Job:** Be lazy about the *solution*, never about the *change*. Ensure callers, fixtures, schema migrations, and tests are completely updated.
4. **Lean Diffs:** Every extra line in a diff increases review friction and token load. Strive for high signal, low noise.
