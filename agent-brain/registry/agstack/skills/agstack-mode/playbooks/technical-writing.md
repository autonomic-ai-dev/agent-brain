# Playbook: Technical Writing & Documentation

Use this playbook whenever you are authoring design documentation, RFCs, READMEs, API guides, pull request descriptions, or commit messages.

---

## Process Steps

### Step 1: Select the Diátaxis Mode Upfront
- Determine the document's single mode before writing:
  - **Tutorial:** Learning by doing (step-by-step commands, visible results).
  - **How-To:** Action-oriented guide for competent engineers.
  - **Reference:** Factual, dry, complete lookup (options, schemas, limits).
  - **Explanation:** Architectural rationale, history, trade-offs, and design context.
- Never mix modes within one document.

### Step 2: Ground in Real Code Vocabulary
- Use real symbol names, exact file paths, configuration keys, and status codes.
- Add clickable markdown file links (`[symbol](file:///path/to/file#L10-L20)`).
- Never use vague descriptions when an exact symbol name exists.

### Step 3: Strip Fluff & Wordiness
- Apply Google Developer Style and Simplified Technical English:
  - Present tense, active voice.
  - Commands as imperatives ("Click Submit", "Run tests").
  - Cut words that do no work ("in order to" -> "to").
  - Eliminate artificial enthusiasm ("simple", "easy", "robust", "blazing-fast").

### Step 4: Review Against Existing Docs
- Follow the formatting, heading levels, and structure of existing documentation in `docs/`.
- Ensure all links and code samples are syntactically valid and tested.
