---
name: principle-never-block-on-human
description: "Interrogate the repository, trace source code, and run empirical experiments. Never stall on questions that the codebase or a test run can answer."
disable-model-invocation: true
---

# Principle: Never Block on the Human

## Invariant
If an answer is observable by reading code, inspecting configuration, checking route tables, or executing a test or prototype, the agent must discover it autonomously. Reserve questions for genuine business/product choices that code cannot settle.

## Application
1. **Interview the Repo, Not the User:**
   - How does the app start? Read package.json, Makefile, Dockerfile.
   - What ports or endpoints exist? Read router source files.
   - What test framework is used? Read build configuration and existing test directories.
2. **Settle Forks Empirically:**
   - When faced with two design choices or performance questions, write a disposable prototype (`playbook: prototype`) and measure the outcome rather than asking "which do you prefer?".
3. **Only External Credentials Justify Asking:**
   - The only valid reason to ask the human is for an external secret or third-party token that cannot be mocked, generated, or derived locally.
