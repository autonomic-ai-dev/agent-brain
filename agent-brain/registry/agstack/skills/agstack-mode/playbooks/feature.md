# Playbook: Feature

Use this playbook whenever you are adding new user-facing, API, CLI, or daemon behavior, or changing existing capabilities.

---

## Process Steps

### Step 1: Interrogate Intent, Constitution & Codebase
- If `CONSTITUTION.md` exists at repository root, read it to ground architecture, technical stack, and negative constraints.
- If a product specification or issue exists (`docs/features/*.md`), map each planned implementation change to an explicit **acceptance criterion**.
- Navigate source files to find router declarations, controllers, CLI command parsers, and configuration loaders.
- Identify existing models, databases, and dependencies. Never ask the user for facts visible in the repository.

### Step 2: Model the Domain & Data Shape
- Define core types, domain models, and boundary schemas (e.g., Zod, serde, Pydantic) before writing business logic.
- Enforce `principle-boundary-discipline`: validate strictly at system boundaries, trust internal typed models.
- Ensure all models are immutable (frozen dataclasses, readonly properties).

### Step 3: Author or Update Feature Map Contract
- Check or create `.verify/features/<feature>.md`.
- Enforce the strict 4-H2 structure:
  1. `## Sub-features`: Kebab-case IDs with intent, behavior, and outcome.
  2. `## How to get to it (user POV)`: Literal CLI, HTTP, or message entrypoints.
  3. `## Verification Suites (Targeted Test Commands)`: Unit command, functional command, allowlisted command, and numbered four-part steps:
     - `Request:` method, path, payload file.
     - `Why:` the behavior proved.
     - `Expected:` status code and concrete assertions with values.
     - `Impact:` final system state changes.
  4. `## Gotchas`: Known operational traps, concurrency bounds, and unconfirmed expectations.
- Update `.verify/features/README.md` index.

### Step 4: Implement Behavior (TDD)
- Write unit tests covering pure business logic first.
- Implement the minimal code required to satisfy the contract.
- Adhere to `principle-immutability-first` and KISS/DRY.

### Step 5: Pair with Functional API Tests (Hurl)
- Create or update `.verify/functional/<feature>.hurl`.
- Pair every request entry with exactly one step in the feature contract (`(entries …)`).
- Ensure payloads exist under `.verify/functional/payloads/`.

### Step 6: Phased Verification & Quality Gates
- Execute the phased runner through the allowlist:
  ```bash
  sh .verify/run.sh <feature>
  ```
- Run static checks including `checks/feature_coverage.sh` to ensure 1:1 step coverage.
- If any gate fails, stop and fix the cause. Never weaken tests.

### Step 7: Clean Hygiene & Evidence Record
- Strip narration comments and temporary scaffolding (`no-comments`).
- Ensure proof artifacts survive in `.verify/artifacts/`.
- Present the evidence record: commands run, exit codes, and measured coverage.
