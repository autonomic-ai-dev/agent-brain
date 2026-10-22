# Playbook: Investigation

Use this playbook whenever you are asked a read-only architecture, flow, or diagnostic question (e.g., "how does X work?", "why is Y failing?", "where is Z configured?").

---

## Process Steps

### Step 1: Autonomous Code Discovery
- Read the relevant build manifests, routing tables, and service entry points.
- Trace execution flow directly through the codebase.
- Do NOT guess, hypothesize without verification, or ask the user for details observable in the repository.

### Step 2: Source Code Evidence Gathering
- Extract exact file paths and line ranges for:
  - Entry point / route handler
  - Core business logic / transformation
  - Persistence / database query / event emission
- Format all code references as clickable markdown links: `[symbol](file:///absolute/path#L10-L20)`.

### Step 3: Empirical Validation
- If the behavior depends on runtime state or environment variables, execute a read-only probe (e.g., query database schema, check doctor check, run `--help`).
- Settle ambiguities with empirical facts, not speculation.

### Step 4: Unslopped, Structured Briefing
- Structure the response with:
  1. Direct answer in the first sentence.
  2. Concrete flow diagram or call sequence.
  3. Evidence table with clickable file links and exact line references.
  4. Known edge cases or operational gotchas.
