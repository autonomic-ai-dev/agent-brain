# Playbook: Design & Architecture

Use this playbook whenever you are introducing new architecture, crossing module boundaries, adding background services, or restructuring data flow.

---

## Process Steps

### Step 1: Align with Existing Repository Design Patterns
- Inspect existing modules in the codebase:
  - How are services structured?
  - How are errors propagated and mapped?
  - How are database connections, transactions, and event topics handled?
- Follow the repository's established conventions strictly. Do not invent foreign patterns.

### Step 2: Domain & Data Modeling First
- Apply `principle-foundational-thinking` and `principle-boundary-discipline`:
  - Model core domain entities as immutable data structures (`principle-immutability-first`).
  - Use discriminated unions or enums to make illegal states unrepresentable.
  - Define schema validation (Zod, serde, Pydantic) for external boundary inputs.

### Step 3: Deep Interface Design
- Design public APIs that are deep: a compact, clean surface hiding necessary internal complexity.
- Avoid wide, shallow interfaces that leak internal implementation details or require callers to execute multi-step orchestration.

### Step 4: Screen for Red Flags
- Screen against speculative abstractions (YAGNI), deep nesting (>4 levels), and signal threading.
- Ensure all resources (locks, sockets, connections) have explicit timeouts and release semantics.

### Step 5: Draft ADR / Design Note (When Consequential)
- If introducing an architectural boundary, write an ADR under `docs/adr/` stating context, options considered, the chosen design, and consequences.
