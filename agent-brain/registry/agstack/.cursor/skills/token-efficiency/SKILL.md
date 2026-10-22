---
name: token-efficiency
description: "Token-efficient engineering and minimal blast radius (inspired by ponytail). Solves the complete task with the least new code and minimal token consumption across context reads and output generations. Rethinks whether new code is needed before writing it."
disable-model-invocation: true
---

# Token Efficiency & Minimal Blast Radius

The best code is the code never written. Every extra line of code must be read, parsed by LLMs, tested, reviewed, and maintained forever. `token-efficiency` equips agents to solve the entire problem with minimal code, smallest blast radius, and optimal token budget.

## 1. Before You Write: Rethink Whether Code is Needed

Before writing a single line of implementation, pause and evaluate this hierarchy (take the first option that fully works):

1. **Does it need to exist?**
   - Skip speculative features, options, and flexibility nobody asked for (YAGNI).
   - If a simpler design eliminates the need for new logic entirely, choose the simpler design.
2. **Already in this codebase?**
   - Search the repo (`grep_search`, symbol lookup). If an existing helper, component, utility, or service already does this, reuse it the way surrounding code does.
3. **Standard library or platform feature?**
   - Use built-in language or runtime capabilities (e.g., native browser date picker, standard library string/date helpers, native collection methods).
4. **Already an installed dependency?**
   - Use what is in `package.json`, `Cargo.toml`, or `go.mod`. Never add a new third-party dependency for a few lines of logic.
5. **Can it be one line a reader gets at a glance?**
   - Write that one line. A clear one-liner beats a multi-file abstraction layer.
6. **Otherwise: The minimum code that completely works.**

## 2. Minimal Blast Radius

- **Keep Touchpoints Minimal:** Avoid touching 10 files when 1 or 2 targeted edits solve the issue completely.
- **Trace Callers Before Editing:** Identify every caller of the function you touch. Fix root causes in shared code once rather than scattering defensive patches across callers.
- **Preserve Unrelated Code:** Never churn whitespace, reformat untouched code, or execute bulk automated edits that inflate the diff and pollute the context window.

## 3. Token-Efficient Context Management & Supervisor Discipline

The game is to reduce token usage across the entire SDLC:
- **Obey `route_task` and `must_apply`:**
  - `agent-brain` acts as the execution supervisor. Always read and strictly obey `must_apply` constraints returned by `route_task`.
  - Load only the recommended skill paths returned; do not grep or load the entire skill library into context.
- **Bounded Inspection Over Full Dumps:**
  - Never dump a 1,000-line file into context just to find a 5-line function.
  - Search symbols first with `grep_search`, then read bounded chunks using line ranges, `read_file_head`, or `read_file_tail`.
  - Never read build output (`dist/`, `target/`, `node_modules/`, `.git/`) or dump raw minified bundles.
- **Persist Anti-Patterns to Prevent Rabbit Holes:**
  - When an agent burns tokens on a false path, identify the trap immediately:
    ```text
    store_memory(
      topic: "token-trap-<name>",
      fact: "Do not read X; use Y on src/ instead.",
      polarity: "negative"
    )
    ```
  - `agent-brain` records this anti-pattern and surfaces it as a `must_apply` hard constraint on future turns.
- **Zero Conversational Fluff:**
  - Speak engineer-to-engineer. Avoid polite filler ("Certainly!", "I'd be happy to help!").
  - Lead directly with answers, code diffs, and evidence records.

## 4. What is Never Cut

Being token-efficient and minimal is **never** an excuse for being sloppy. You must NEVER cut:
- Validation at system trust boundaries (`principle-boundary-discipline`).
- Error handling that prevents data loss, resource leaks, or crashes.
- Security checks, authentication, and input sanitization.
- Meaningful tests: new non-trivial logic must leave behind focused unit and functional tests.
