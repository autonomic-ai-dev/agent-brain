---
name: verification
description: "Set up and run a repository's verification harness in .verify/: an allowlist of trusted commands, a phased runner (doctor, unit, static, functional), Hurl functional suites, and proof artifacts. Use whenever code changes need evidence that they pass, or when the harness must be created."
disable-model-invocation: true
---

# Verification

A change is not finished until the repository's own harness shows that it passes its unit checks and its functional contracts. The harness lives under `.verify/`. Only the commands it allowlists count as evidence.

Pair this skill with the feature-map skill: the feature map says which tests prove each behaviour, and this skill runs them.

## Load the reference you need

| Task | Load |
|---|---|
| Set up or change the allowlist, the runner, or the doctor | `references/allowlist-and-runner.md` |
| Write or review a Hurl functional suite | `references/hurl-standards.md` |

## Invariants

These rules apply whenever you run or change verification:

1. **Read the repository first.** Find the test commands, endpoints, and environment settings from the repository's own files.
2. **Run through the allowlist.** Run verification through `.verify/run.sh`, or another entry point listed in `.verify/verify.toml`. A command outside the allowlist is not evidence.
3. **Check the environment before testing.** When `doctor.sh` fails, stop and fix the environment. Do not drive tests against a broken setup.
4. **Stop only what you started.** End background processes by the process IDs the run recorded. Do not kill processes by name or by pattern.
5. **Keep the artifacts.** Logs, reports, and responses under `.verify/artifacts/` survive teardown. Cleanup never deletes them.
6. **Fix failures, never weaken them.** Do not comment out a failing test, loosen an assertion, or remove a command from the allowlist to make a run pass. Fix the cause, or record it as a known failure with its evidence.
7. **Report what ran.** State the commands run, their results, and anything that did not run, and why.
