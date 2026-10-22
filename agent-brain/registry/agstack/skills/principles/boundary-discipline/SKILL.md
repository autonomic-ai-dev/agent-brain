---
name: principle-boundary-discipline
description: "Parse and validate untrusted external data strictly at system boundaries; trust internal types and keep domain logic pure."
disable-model-invocation: true
---

# Principle: Boundary Discipline

## Invariant
Validate and parse all external data at the edge of the system (HTTP controllers, CLI arguments, event listeners, database readers). Once inside the boundary, trust internal strongly-typed models without defensive re-validation.

## Application
1. **Parse, Don't Validate:** Use schemas (Zod, serde, Pydantic, protobuf) to transform raw input into typed internal domain objects. If input is malformed, reject it immediately with structured error messages.
2. **Pure Domain Logic:** Domain functions should accept clean, typed parameters and return typed results. They should not re-check nullability or string formats already validated at the boundary.
3. **Fail Fast:** Never allow unparsed JSON, untyped maps, or loose primitives to wander through multiple layers of business logic.
