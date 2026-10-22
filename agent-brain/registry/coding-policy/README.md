# Strict Coding-Agent Policy & Verification Bundle

A unified engineering bundle for Cursor, Claude Code, Antigravity CLI (`agy`), OpenCode, and Augment Code.

Combines three pillars into a single bundle to help autonomous agents write clean, idiomatic, fully tested code:
1. **Strict Coding Policy:** Immutability, native tools first (no Python as shell/reader workaround), no unnecessary comments, minimal diffs, least privilege.
2. **Feature Mapping (`@feature-map`):** Behavior contracts under `.verify/features/` capturing sub-features, user entry points, and known gotchas—linked directly to test commands.
3. **Test Verification Harness (`@verification`):** `.verify/verify.toml` allowlist, `.verify/run.sh` composite runner, unit tests, and declarative Hurl API functional suites under `.verify/functional/`.

---

## What is in this bundle

```text
coding-policy/
├── AGENTS.md                  # Canonical shared policy across all agents
├── .cursor/
│   ├── rules/
│   │   ├── coding-policy.mdc  # Core engineering policy rule
│   │   ├── feature-map.mdc    # Feature map policy (.verify/features/)
│   │   └── verification.mdc   # Test verification policy (verify.toml + Hurl)
│   └── skills/                # the skills; installed as-is into each agent's skill location
│       ├── feature-map/
│       │   ├── SKILL.md       # entry point: which reference to load for which task
│       │   ├── references/    # contract format, authoring rules, layout and gates
│       │   └── scripts/       # entry-coverage check, coverage floor, gate report
│       └── verification/
│           ├── SKILL.md       # entry point and invariants
│           └── references/    # allowlist and runner, Hurl standards
├── antigravity/AGENTS.md      # Antigravity CLI instructions
├── claude/CLAUDE.md           # Claude Code instructions
├── opencode/AGENTS.md         # OpenCode instructions
├── augment/AGENTS.md          # Augment Code instructions
├── scripts/
│   ├── check-added-comments.sh
│   └── install-git-hook.sh
└── .github/workflows/
    └── no-added-comments.yml
```

---

## Install

Copy each platform file to the location your agent host reads, as listed in the file tree above. A host package manager that supports this bundle can install it in the same way.

For a repository that uses the verification harness, create `.verify/` with `verify.toml`, `run.sh`, `doctor.sh`, `features/`, and `functional/` as described in the feature-map and verification skills.

---

## Workflow in Repositories

1. **Before Modifying Code:**
   - Review relevant contracts in `.verify/features/<feature>.md`.
   - Read `## How to get to it (user POV)` and `## Gotchas`.
2. **Implement Code:**
   - Adhere to native tools, strict immutability, and no stray comments.
3. **Execute Linked Verification:**
   - Run the exact targeted unit and Hurl test commands specified under `## Verification Suites`, through the allowlist in `.verify/verify.toml`:
     ```bash
     sh .verify/run.sh <feature>
     ```
4. **Preserve Proof:**
   - Ensure all output logs and reports are captured under `.verify/artifacts/`.
