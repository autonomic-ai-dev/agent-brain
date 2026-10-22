---
name: principle-immutability-first
description: "Always return new objects and collections rather than mutating existing ones in place. Eliminates hidden side effects and race conditions."
disable-model-invocation: true
---

# Principle: Immutability First

## Invariant
Never mutate objects, arrays, or records in place. Functions must accept inputs, perform transformations, and return fresh copies.

## Application
1. **Return Updated Copies:**
   - In TypeScript/JavaScript: Use object/array spread (`{ ...obj, key: value }`, `[...items, newItem]`) or `Object.assign`.
   - In Python: Use frozen dataclasses (`@dataclass(frozen=True)`), `NamedTuple`, or mapping copies.
   - In Rust: Use `let` by default; avoid `let mut` unless strictly necessary for local accumulator performance.
   - In Go/JVM: Return updated struct copies; prefer `val` in Kotlin, `record` / `final` in Java.
2. **Predictable Concurrency & Debugging:**
   - Immutable data prevents spooky action-at-a-distance bugs where a background thread or downstream subscriber alters state unexpectedly.
