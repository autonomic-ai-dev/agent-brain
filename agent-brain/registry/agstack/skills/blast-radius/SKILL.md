---
name: blast-radius
description: "Analyze transitive impact of changes across callers, schemas, event streams, and downstream services. Prove safety with running code rather than assumptions."
disable-model-invocation: true
---

# Blast Radius: Impact Analysis

A small change can trigger subtle cascading failures across callers, database queries, serialized contracts, and background tasks. `blast-radius` ensures changes are safe before they land.

## The Blast Radius Protocol

### 1. Caller & Interface Tracing
- Identify every direct and transitive caller of the modified function, type, or module using `find_callers`, `grep_search`, or LSP symbol references.
- Check for dynamic invocations, reflection, serialized JSON interfaces, or message bus topics where static analyzers cannot trace callers.
- Verify whether the change modifies parameter order, defaults, optionality, or return shapes.

### 2. Data & Schema Dependencies
- If a database column, table, or migration is touched:
  - Check for old clients and rolling deployment compatibility (can version N-1 read version N's database?).
  - Ensure every migration includes a tested rollback path.
  - Verify index usage: ensure modified queries do not trigger full table scans.

### 3. Concurrency & Resource Boundaries
- Check if the modified path acquires locks, opens sockets, or consumes memory buffers.
- Ensure all resources are released in error and timeout paths.
- Ensure retries are bounded with backoff and jitter, and applied only to idempotent operations.

### 4. Prove Safety with Running Code
- Run the full suite of downstream tests that exercise the callers:
  ```bash
  sh .verify/run.sh all
  ```
- If downstream tests do not cover the affected callers, author new unit or integration tests before proceeding.
- Safety is proven by running code and green exit codes, never by assertion.
