---
name: architect
description: "Design system architecture, domain models, and interfaces before implementing code. Grounds decisions in existing repository patterns, ADRs, and caller ergonomics. Evaluates interface depth and screens for over-engineering."
disable-model-invocation: true
---

# Architect: Pre-Implementation Design & Modeling

Before writing implementation logic that crosses module or function boundaries, ground the architecture in the existing codebase and establish clean, deep interfaces.

## The Architecture Lifecycle

### Phase 1: Ground in the Existing System
- **Study Existing Design Patterns:** Inspect the repository's existing modules, service layers, and patterns. If the codebase uses repository-service-controller, actor models, clean architecture, or functional pipelines, adhere to that established pattern. Never invent a foreign architecture just because it is familiar.
- **Trace Design Docs & ADRs:** Read `docs/`, `rfcs/`, `adr/`, and previous PR descriptions to understand architectural decisions and constraints.
- **Model the Domain First:** Define core types, data shapes, and state transitions before control flow. Make illegal states unrepresentable (`principle-foundational-thinking`).

### Phase 2: High-Level Design (HLD) with Mermaid Diagrams
- Every substantial architectural design must include an HLD diagram:
  - **Component Flowchart:** Illustrates system boundaries, controllers, services, databases, and message brokers.
  - **Sequence Diagram:** Traces request/response lifetimes, asynchronous worker dispatches, and timeout gates.
- Always validate Mermaid syntax: quote labels with special characters, use standard flowchart (`graph TD` / `flowchart LR`) or sequence (`sequenceDiagram`) types.

### Phase 3: Low-Level Design (LLD)
- Detail the low-level contracts:
  - **Schemas & Invariants:** Boundary validation schemas (Zod/serde) and domain invariants.
  - **Interface Depth & Caller Ergonomics:** Deep interfaces hiding internal complexity behind a compact public surface.
  - **Error Models:** Explicit failure modes, domain errors, and recovery strategies.
  - **Concurrency & Resource Lifecycle:** Lock acquisition order, timeout bounds, and release semantics.

### Phase 4: Screen Against Red Flags
- Screen the design before writing code:
  - ❌ Speculative abstraction: interfaces with only one foreseeable implementation (YAGNI).
  - ❌ Leaky abstractions: internal implementation details leaking into public signatures.
  - ❌ Deep nesting: more than 3-4 levels of hierarchy.
  - ❌ Excessive signal threading: passing unnecessary context objects through layers that do not use them.

### Phase 5: Implementation Against the Contract
- Implement against the approved architectural sketch.
- Deviations during implementation are signals to re-evaluate the architecture, not friction to absorb silently.
