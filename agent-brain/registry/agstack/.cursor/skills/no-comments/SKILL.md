---
name: no-comments
description: "Enforce strict comment hygiene on newly written or modified code. Strips AI narration, inline chatter, and commented-out dead code while strictly preserving existing documentation, public API contracts, and license headers."
disable-model-invocation: true
---

# No Comments & Comment Hygiene

AI coding agents tend to generate noisy, redundant comments that narrate the obvious, apologize for workarounds, or leave dead code behind. This skill enforces ruthless comment discipline.

## The Rule

1. **Maintain Documentation Integrity:**
   - Preserve all existing comments, docstrings, and architectural notes in untouched code.
   - Do NOT mass-delete comments from existing files unless explicitly instructed by the user.

2. **Zero AI Narration in New Code:**
   - Never add inline comments that merely describe what the code is doing (`// increment counter`, `// fetch user from database`, `// handle error`).
   - If code needs an explanation to be understood, rename symbols, extract a helper, or clarify types so the code explains itself.

3. **Zero Commented-Out Dead Code:**
   - Never leave commented-out blocks of old code, obsolete functions, or deprecated logic. If code is no longer needed, delete it cleanly. Git preserves history.

4. **Zero Workaround Sermons:**
   - Never leave comments like `// temporary workaround for now`, `// fix later`, `// hack: should be refactored`.
   - Fix the root cause immediately, or record the constraint in the feature contract's `Gotchas`.

5. **Permitted Exceptions (What May Stay):**
   - Legal, copyright, and license headers.
   - Public API documentation comments (e.g., JSDoc, Rustdoc, Godoc) that define external contracts, parameter schemas, and return semantics.
   - External protocol or vendor quirks that cannot be reshaped and are genuinely non-obvious.
   - Compiler/linter directives (`@ts-expect-error`, `nolint`) when strictly necessary and accompanied by a brief rationale.

## Enforcement

- Pre-commit hook: `scripts/check-added-comments.sh` checks staged diffs for newly introduced narration comments.
- Before completing any task, review the diff against `master` and strip any newly added conversational comments.
