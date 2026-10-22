# agstack (Abhiuday Gupta Stack)

> If you want to ship reliable software, turn engineering discipline into an unbreakable process.

**`agstack`** is an agent-agnostic engineering operating system for autonomous coding agents (Antigravity CLI, Cursor, Claude Code, OpenCode, Codex, GitHub Copilot). It replaces prompt guessing with structured playbooks, foundational principles, four-part step verification (`.verify/`), and ruthless code hygiene.

---

## 1. Why agstack?

AI agents without a strict process produce slop: commentary that restates obvious code, fragile workarounds, unverified claims, and hallucinated fixes. 

Inspired by [pstack](https://github.com/cursor/plugins/tree/main/pstack), **`agstack`** adapts rigorous software practices into an agentic framework:
- **Full-SDLC Playbooks (`agstack-mode`):** Dedicated step-by-step procedures covering every lifecycle phase: `context-and-intent`, `design-and-architecture`, `bug-fix`, `feature`, `refactoring`, `test-suite-generation`, `security-and-jailbreak-defense`, `code-review`, `technical-writing`, `prototype`, and `autonomous-run`.
- **Interrogate Code, Never the User:** Agents discover entry points, ports, and schemas by navigating the codebase.
- **Deep Architecture & Patterns:** Conforming strictly to the repository's existing architecture, ADRs, and caller ergonomics (`architect`).
- **Forensics & Bug Repro:** Diagnosing stack traces, parsing logs, and proving bugs with failing runtime reproductions *before* writing any fix (`forensics`).
- **Blast Radius Analysis:** Proving safety across transitive callers, schema migrations, and event streams with running code (`blast-radius`).
- **Foundational Principles:** Grounding in immutability, boundary validation (Zod/serde), root-cause fixes, and testing observable behavior.
- **Four-Part Verification Harness (`.verify/`):** 4-H2 feature contracts with four-part step records (`Request:`, `Why:`, `Expected:`, `Impact:`), allowlisted commands (`verify.toml`), phased runners (`run.sh`), and generic coverage checks (`checks/feature_coverage.sh`).
- **Code Hygiene (`no-comments`, `unslop`):** Zero AI narration comments, zero commented-out code corpses, and concise, idiomatic code and prose.
- **High-Quality Technical Writing:** Clear, layered technical documentation following the Diátaxis framework, Google developer style, and Simplified Technical English (`technical-writing`).

---

## 2. Directory Layout

```text
agstack/
├── package.json                         # Universal bundle manifest
├── README.md                            # agstack documentation
├── AGENTS.md                            # Universal master engineering policy
├── FEATURE-MAP-FORMAT.md                # 4-H2 and 4-part verification contract spec
│
├── platforms/                           # Multi-agent platform projections
│   ├── claude/CLAUDE.md                 # Claude Code adapter
│   ├── github/copilot-instructions.md   # GitHub Copilot adapter
│   └── cursor/rules/                    # Cursor rules
│       ├── agstack.mdc                  # Core policy
│       ├── feature-map.mdc              # Feature map rules
│       ├── verification.mdc             # Verification rules
│       └── principles.mdc               # Principles reference
│
├── skills/                              # Universal skills (Open Agent Skills Standard)
│   ├── agstack-mode/                    # Playbook orchestrator
│   │   ├── SKILL.md
│   │   └── playbooks/                   # bug-fix, feature, refactoring, etc.
│   ├── feature-map/                     # Feature contract authoring & coverage
│   │   ├── SKILL.md
│   │   ├── references/                  # contract-format, authoring-rules, layout-and-gates
│   │   └── scripts/                     # feature_coverage.sh, coverage_gate.sh, gate_report.sh
│   ├── verification/                    # Verification harness runner & allowlist
│   │   ├── SKILL.md
│   │   └── references/                  # allowlist-and-runner, hurl-standards
│   ├── security-audit/                  # Vulnerability scanning, secrets, OWASP, AST validation
│   ├── jailbreak-defense/               # Prompt injection defense, delimiters, tool bounds
│   ├── workflow-engine/                 # Autonomous DAG workflow self-orchestrator (zero-Spine)
│   ├── no-comments/                     # Strip AI chatter, preserve documentation integrity
│   ├── unslop/                          # Concise, direct, idiomatic code & prose
│   ├── interrogate/                     # Adversarial pre-completion review
│   └── principles/                      # Core engineering principles
│       ├── foundational-thinking/
│       ├── boundary-discipline/
│       ├── immutability-first/
│       ├── test-behavior-not-implementation/
│       ├── fix-root-causes/
│       └── never-block-on-human/
│
├── .cursor/                             # Cursor-native mirrors
└── scripts/
    ├── check-added-comments.sh          # Git pre-commit comment linter
    ├── install-git-hook.sh              # Hook installer
    ├── run-workflow.py                  # Portable DAG workflow engine
    └── run-workflow.sh                  # Shell wrapper for standalone workflow execution
```

---

## 3. Installation & Usage

### Via Agent-Brain (Recommended)
```bash
agent-brain package add @agstack
```

### In Cursor
Pick `agstack-mode` or run with the `.cursor/rules/agstack.mdc` active by default.

### In Claude Code
`platforms/claude/CLAUDE.md` is automatically installed by `agent-brain install --host claude` or by copying to `CLAUDE.md`.

### In Antigravity CLI / OpenCode / Codex
`AGENTS.md` and `skills/` conform to the native Agent Skills specification and are discovered automatically.

---

## 4. Verification Harness (`.verify/`)

When working in a repository with `.verify/`:
1. **Pre-flight Check:** Run `.verify/doctor.sh`.
2. **Execute Targeted Verification:**
   ```bash
   sh .verify/run.sh <feature>
   ```
3. **Verify Coverage:** `checks/feature_coverage.sh` guarantees 1:1 parity between feature contract steps and `.hurl` requests.
4. **Preserve Proof:** Proof artifacts in `.verify/artifacts/` survive teardown.
