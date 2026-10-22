---
name: security-audit
description: "Comprehensive vulnerability auditing for source code, dependencies, credentials, secrets, memory safety, and OWASP Top 10 vulnerabilities."
disable-model-invocation: true
---

# Security Audit: Vulnerability & Hardening Standards

Security is not a post-release afterthought; it is an active engineering constraint across the entire SDLC. `security-audit` establishes mandatory vulnerability scanning, secret detection, dependency audits, and defensive remediation patterns.

---

## 1. Vulnerability Checklist & Audit Protocols

### A. Secret & Credential Hygiene
- **Zero Secrets in Code:** Never commit passwords, API keys, private certificates, OAuth tokens, or JWT signing secrets to source code or git history.
- **Fail-Fast Environment Validation:** All required environment variables and secrets must be parsed and strictly validated at service boot using a validated schema (e.g., Zod, Pydantic, envy). Services must fail to start immediately if any secret is missing or malformed.
- **Git History Hygiene:** If a secret was ever committed, it must be rotated immediately in production and wiped from repository history using git-filter-repo.

### B. Input Validation & Injection Defenses (OWASP Top 10)
- **SQL / NoSQL Injection:**
  - Mandatory use of parameterized queries, prepared statements, or strongly typed ORM query builders.
  - String concatenation or template literals in database queries are strictly prohibited.
- **Command & Code Injection:**
  - Avoid raw shell execution (`sh -c`, `system()`, `eval()`).
  - When spawning subprocesses, pass arguments as discrete array elements (e.g., `execFile`, `Command::new("bin").args([...])`).
  - Pass commands through AST validators (e.g., `agent-mouth` / `mouth_validate_ast`) before execution.
- **Path Traversal:**
  - Sanitize all file paths from user inputs.
  - Canonicalize paths using `filepath.Clean`, `path.resolve`, or `std::fs::canonicalize` and verify that the target path resides strictly within the intended base directory. Reject paths containing `..`.
- **Server-Side Request Forgery (SSRF):**
  - Whitelist permitted destination hosts and schemes.
  - Block internal private IP ranges (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `127.0.0.1/8`, `::1`) and cloud metadata endpoints (`169.254.169.254`).
- **Cross-Site Scripting (XSS):**
  - Context-aware HTML entity encoding on all user-supplied output.
  - Strict Content-Security-Policy (CSP) headers.

### C. Dependency & Supply Chain Auditing
Run automated dependency auditing in CI and `.verify/run.sh`:
- **Rust:** `cargo audit`
- **Node/TypeScript:** `npm audit --audit-level=high` or `pnpm audit`
- **Python:** `pip-audit`
- **Go:** `govulncheck ./...`
- Verify that lockfiles (`Cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`, `poetry.lock`) are committed and tamper-checked.

### D. Concurrency, Memory Safety & Cryptography
- **Memory Safety:**
  - Rust: Ban `unsafe` blocks unless accompanied by a written invariant proof and sound safety comments.
  - C++: Use RAII, smart pointers (`std::unique_ptr`), and bounds-checked spans (`std::span`).
- **Cryptography:**
  - Cryptographically secure pseudo-random number generators (CSPRNG) only (e.g., `crypto.getRandomValues`, `rand::rngs::OsRng`). Never use `Math.random()`.
  - Constant-time comparisons (`crypto.timingSafeEqual`, `subtle.constant_time_eq`) for hashes, HMACs, and security tokens to prevent side-channel timing attacks.
  - Modern password hashing: Argon2id, scrypt, or bcrypt with appropriate cost factors.

---

## 2. The 4-Part Vulnerability Finding Schema

When conducting a security review or audit, format every discovered issue using this standard:

```markdown
### [MUST FIX] [CWE-89] SQL Injection in `getUserByEmail`

- **What this is:** Direct string interpolation in SQL query at `src/db/users.ts#L42`.
- **Threat Vector / CWE:** CWE-89 (Improper Neutralization of Special Elements used in an SQL Command). An attacker can inject SQL via the `email` query parameter to dump user table contents.
- **Defensive Remediation:** Replace string template with parameterized query:
  ```typescript
  // Before: db.query(`SELECT * FROM users WHERE email = '${email}'`);
  // After:
  db.query("SELECT * FROM users WHERE email = $1", [email]);
  ```
- **Proof of Fix / Verification:** Added regression test in `tests/security/sql_injection_test.ts` passing `' OR '1'='1` and asserting only the matching user is returned.
```

---

## 3. Automated Security Verification in `.verify/`

Every project repository must integrate security checks into `.verify/run.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# 1. Dependency Vulnerability Audit
if command -v cargo-audit &>/dev/null && [ -f Cargo.toml ]; then
  cargo audit
fi
if [ -f package.json ]; then
  npm audit --audit-level=high
fi

# 2. Secret Scan
# Check for uncommitted secret patterns
if command -v gitleaks &>/dev/null; then
  gitleaks protect --verbose --staged
fi

# 3. Security Unit & Regression Suite
if [ -f tests/security_test.sh ]; then
  sh tests/security_test.sh
fi
```
