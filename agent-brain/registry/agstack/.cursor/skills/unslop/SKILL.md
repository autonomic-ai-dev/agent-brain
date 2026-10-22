---
name: unslop
description: "Eliminate AI slop, bloated boilerplate, and excessive abstractions. Enforces direct, concise, idiomatic code and crisp, evidence-based reporting without fluff or generic filler."
disable-model-invocation: true
---

# Unslop: Concise, Idiomatic, Product-Ready

AI-generated code and text frequently suffer from "slop" — unnecessary abstraction layers, boilerplate wrappers, defensive fluff, and robotic cheerleading. `unslop` strips this away to produce professional, clean engineering.

## In Code

1. **No Speculative Abstractions (YAGNI):**
   - Do not create generic factory interfaces, strategy patterns, or visitor handlers for a single implementation.
   - Build what is required now. Keep abstractions flat (maximum 3-4 levels of indirection).

2. **No Defensive Hallucinations:**
   - Do not add chains of redundant null checks or fallback defaults for variables that internal types guarantee exist.
   - Parse at boundaries (`principle-boundary-discipline`); trust internal invariants.

3. **Collapse One-Caller Wrappers:**
   - If a private function is only called once from a single place and merely delegates a call, inline it.
   - Keep file lengths focused (< 400 lines typical, 800 lines absolute maximum).

4. **Idiomatic Language Features:**
   - TypeScript: Use discriminated unions, strict types, and standard library methods (`map`, `filter`, `reduce`) over custom loop gymnastics.
   - Rust: Use iterator chains, `?` operator, and RAII rather than deeply nested matches.
   - Python: Use list/dict comprehensions, dataclasses, and context managers.
   - Go: Keep interfaces small (1-3 methods), return concrete structs, accept interfaces.

## In Prose & Reports

1. **Lead with Answers, Not Politeness:**
   - No robotic openings: avoid "Certainly!", "I'd be happy to help!", "Great question!".
   - State the result, the problem, or the conclusion in the very first sentence.

2. **No Hand-Waving Claims:**
   - Never say "the tests should pass now" or "it looks good".
   - State the command run, the exit code, and the output proof.

3. **No Fluff Summaries:**
   - Avoid re-explaining common programming concepts. Speak engineer-to-engineer.
   - Provide clickable file links (`[file.ts](file:///path/to/file.ts#L10-L20)`) and precise metrics.
