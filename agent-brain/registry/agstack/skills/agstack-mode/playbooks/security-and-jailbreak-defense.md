# Playbook: Security Audit & Jailbreak Defense

Use this playbook whenever implementing features that handle user inputs, untrusted external content, tool invocations, or when conducting security audits before releasing code.

---

## Process Steps

### Step 1: Threat Modeling & Injection Surface Mapping
- **Map all untrusted ingress points:**
  - Direct user inputs (chat prompts, search queries, form submissions).
  - Indirect data sources (web scraping, git diffs, uploaded documents, database fields, webhook payloads).
  - External tool responses (third-party APIs, search engine outputs).
- **Classify potential attack vectors:**
  - Prompt injection & jailbreaks (LLM01): Can input override agent instructions?
  - Sensitive information leakage (LLM02): Can system prompt, keys, or credentials be extracted?
  - Excessive tool agency (LLM06): Does the model have permissions that could cause permanent data loss or privilege escalation?
  - Codebase injection (CWE-89 SQL, CWE-78 Command, CWE-22 Path Traversal).

### Step 2: Implement Defensive Isolation & Hardening
- **Segregate Data from Instructions:**
  - Wrap all dynamic inputs in strict delimiters (e.g. `<untrusted_user_input>`).
  - Escape internal delimiter instances in untrusted text.
  - Anchor system prompts with non-override clauses.
- **Enforce Boundary Schemas & Safe APIs:**
  - Validate all inputs and tool parameters with strict runtime schemas (Zod, Pydantic, Serde).
  - Use parameterized queries for all database interactions.
  - Avoid raw shell invocations; use argument vectors and validate AST with `agent-mouth`.
  - Apply the principle of least privilege: gate destructive tool calls behind approval gates (`mouth_request_approval`).

### Step 3: Author Adversarial Test Suites in `.verify/`
- Before marking code secure, write deterministic automated regression tests:
  - Test prompt injection bypass attempts: assert that adversarial inputs inside delimiters are treated as literal strings and not instructions.
  - Test delimiter escape attempts: verify `</untrusted_user_input>` sequences are escaped or neutralized.
  - Test boundary validation: assert that invalid paths, shell metacharacters, and SQL injection strings produce validation errors.
  - Verify that system prompt secrets and environment variables are never echoed in outputs.

### Step 4: Security Gate & Audit Findings Report
- Run security auditing tools:
  ```bash
  # Run dependency audit and security verification
  sh .verify/run.sh security
  ```
- If vulnerabilities are discovered, document them using the 4-part schema:
  - **What this is:** Exact file, line, and trigger condition.
  - **Threat Vector / CWE / OWASP:** Category and risk.
  - **Defensive Remediation:** Concrete code fix.
  - **Proof of Fix:** Passing test in `.verify/`.
- Verify zero security regressions before merging.
