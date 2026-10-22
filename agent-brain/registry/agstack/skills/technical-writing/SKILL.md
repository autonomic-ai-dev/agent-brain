---
name: technical-writing
description: "Layered technical documentation standard (Diátaxis framework, Google developer style, Simplified Technical English). Use when writing design docs, RFCs, READMEs, API specifications, and pull request descriptions."
disable-model-invocation: true
---

# Technical Writing: Clear, Layered, Undebloated

Write so a tired engineer understands on the first read. Good technical documentation informs action or builds clear mental models without wasting the reader's time.

## 1. The Core Rules

- **Cut Every Word That Does No Work:**
  - "In order to" -> "To"
  - "It is important to note that" -> delete
  - "Utilize" -> "Use"
  - "Facilitate" -> "Help"
  - "Perform verification on" -> "Verify"
- **The Codebase is the Vocabulary:** Use literal symbol names, real file paths, CLI flags, and exact status codes. Never use vague paraphrases or generic synonyms.
- **Clickable File Links:** Link directly to files and line ranges using `[symbol](file:///path/to/file#L10-L20)`.

## 2. Diátaxis Framework (One Mode Per Document)

Every technical document must choose its mode upfront based on its purpose:

| Mode | Purpose | Target Reader Need | Structure |
|---|---|---|---|
| **Tutorial** | Learning by doing | Beginner building their first working example | Step-by-step commands, early visible results, minimal theory. |
| **How-To** | Steps to a goal | Competent engineer solving a real task | Action-oriented steps, no digressions, assumes competence. |
| **Reference** | Facts for lookup | Engineer looking up options, schemas, or syntax | Dry, complete, factual, mirrors the code structure. |
| **Explanation** | Understanding & why | Engineer seeking architectural context & trade-offs | Design rationale, historical context, alternatives weighed. |

Never mix modes: no reference tables inside tutorials; no hand-holding inside reference docs.

## 3. Style & Syntax Standards

- **Present Tense & Active Voice:** "The worker pulls the job from NATS", not "The job will be pulled by the worker".
- **Instructions as Imperatives:** "Click Submit", "Run the migration", "Export the environment variable".
- **Condition Before Action:** "To reset the database, run `make db-reset`", not "`make db-reset` should be run if you want to reset the database".
- **Zero Fluff & Artificial Enthusiasm:** Avoid "simple", "easy", "just", "simply", "robust", "blazing-fast", or "revolutionary". State the technical characteristics and benchmark numbers plainly.
