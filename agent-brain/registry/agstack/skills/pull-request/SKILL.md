---
name: pull-request
description: "Author rich, reviewable pull requests and push atomic chunked commits with descriptive commit messages. Structures PRs with Problem Intent, High-Level Design (HLD with Mermaid charts), Low-Level Design (LLD), Blast Radius, and Evidence Records."
disable-model-invocation: true
---

# Pull Request & Delivery Standard

A pull request is an engineering hand-off. It must provide everything a reviewer needs to verify correctness, understand architecture, and assess risk without wading through monolithic, unexplained diffs.

---

## 1. Atomic Chunked Commits

Never push a giant, monolithic commit containing unrelated changes. Push changes in logical, reviewable chunks:

1. **One Purpose per Commit:**
   - Commit 1: Architecture scaffolding / types / schemas (`feat(scope): define domain contracts`).
   - Commit 2: Core implementation logic (`feat(scope): implement state transition`).
   - Commit 3: Tests & verification suites (`test(scope): add unit tests and hurl scenarios`).
   - Commit 4: Documentation & feature map index (`docs(scope): document feature contract`).
2. **Descriptive Conventional Commit Messages:**
   Format: `<type>(<scope>): <short descriptive summary>`
   - Types: `feat`, `fix`, `refactor`, `test`, `docs`, `perf`, `chore`.
   - Body: 1-2 short sentences explaining *why* the change was made and any non-obvious trade-off.

---

## 2. Rich Pull Request Description Template

Every PR description authored under `agstack` must follow this structure:

```markdown
## Summary & Problem Intent
- **What:** One or two sentences summarizing the user-observable change.
- **Why:** The concrete business or technical problem this PR solves (the problem intent).

## High-Level Design (HLD)
System boundaries, component relationships, and execution flow:

\`\`\`mermaid
flowchart TD
    Client["Client / API Gateway"] --> Controller["Service Controller"]
    Controller --> Validator["Boundary Validator (Zod/serde)"]
    Validator --> Logic["Pure Business Domain"]
    Logic --> Persistence["Database / Message Stream"]
\`\`\`

## Low-Level Design (LLD)
- **Data Contracts & Schemas:** Named data shapes and boundary validation rules.
- **Interfaces & Signatures:** Key functions and public API changes.
- **Error Handling:** How failure modes are captured, typed, and recovered.
- **Migration & Rollback:** Tested rollback path for database or configuration migrations.

## Atomic Commit Log
- \`feat(auth): define session token schema and invariants\`
- \`feat(auth): implement token rotation with bounded jitter\`
- \`test(auth): add unit test matrix and functional hurl contract\`
- \`docs(auth): update .verify/features/session-management.md\`

## Blast Radius & Risk Assessment
- **Callers Checked:** List of direct and transitive callers audited.
- **Downstream Impact:** Zero breaking changes on existing clients (backward compatible).
- **Residual Risk:** Any operational gotcha or unconfirmed live value noted in feature contract.

## Security & Prompt Defense Audit
- **Secret Scanning:** Zero hardcoded credentials or keys committed.
- **Jailbreak & Prompt Injection Defense:** All untrusted user and external content isolated inside `<untrusted_input>` delimiters; escaping collisions handled.
- **Boundary Validation:** Parameterized queries used; shell commands vector-passed and AST validated.
- **Dependency Audit:** Zero high or critical vulnerabilities (`cargo audit` / `npm audit`).

## Verification & Evidence Record
Verification executed through the allowlisted harness (\`.verify/run.sh\`):

| Phase | Command | Exit Code | Result | Artifact Path |
|---|---|---|---|---|
| **Doctor** | \`bash .verify/doctor.sh\` | 0 | PASSED | \`.verify/artifacts/<ID>/doctor.log\` |
| **Unit** | \`cargo test --lib\` | 0 | PASSED | \`.verify/artifacts/<ID>/unit.log\` |
| **Static** | \`sh .verify/checks/feature_coverage.sh\` | 0 | PASSED | \`.verify/artifacts/<ID>/coverage.log\` |
| **Functional**| \`hurl --test .verify/functional/auth.hurl\`| 0 | PASSED | \`.verify/artifacts/<ID>/functional.log\` |

- **Line Coverage:** 88.4% (above 80% threshold).
- **Branch Coverage:** 84.1%.
```
