---
name: principle-test-behavior-not-implementation
description: "Verify observable outcomes, return values, and real boundaries instead of internal private methods, mock trivia, or implementation details."
disable-model-invocation: true
---

# Principle: Test Behavior, Not Implementation

## Invariant
Tests must prove what the user or consumer observes (outputs, status codes, database state, event emissions). Do not couple tests to private methods or internal variable names.

## Application
1. **Mock at Genuine Boundaries Only:**
   - Mock external networks, third-party payment gateways, or clock providers. Never mock the very business logic or internal service the test is verifying.
2. **Resilience to Refactoring:**
   - A good test suite passes without changes when internal implementation details are completely refactored.
3. **No Coverage Vanity:**
   - Do not write trivial assertions (e.g. testing getters/setters) to artificially inflate coverage percentages. Test real failure modes, edge conditions, and boundaries.
