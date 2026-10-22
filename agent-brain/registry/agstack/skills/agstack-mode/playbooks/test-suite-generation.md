# Playbook: Test Suite Generation & Maintenance

Use this playbook whenever code changes require new or updated test suites, or when establishing verification for an unmapped feature.

---

## Process Steps

### Step 1: Detect Changed Code Surfaces
- Inspect git diff against `master`:
  - Identify modified functions, routes, parameters, error cases, and database migrations.
  - Determine which public behaviors or invariants have changed.

### Step 2: Author Focused Unit Tests
- Ground in `principle-test-behavior-not-implementation`:
  - Write unit tests covering pure business logic, calculations, and branch conditions.
  - Test boundary values, empty cases, and error mappings.
  - Mock only at genuine system boundaries (external HTTP services, clocks). Never mock the internal logic under test.

### Step 3: Author Declarative Functional Hurl Scenarios
- For HTTP/API surfaces, create or update `.verify/functional/<feature>.hurl`.
- Structure scenarios to validate end-to-end user journeys:
  - Assert HTTP status codes.
  - Assert response headers and JSON body paths (`jsonpath "$.status" == "ok"`).
  - Capture generated IDs (`[Captures] id: jsonpath "$.id"`) and reuse in downstream steps.
  - Clean up created resources (`DELETE ...`) at the end of the scenario.

### Step 4: Map Steps to Feature Contract (1:1 Parity)
- Update `.verify/features/<feature>.md` under `## Verification Suites`:
  - Number each step in entry order.
  - Add the `(entries …)` range in each step heading.
  - Provide all four lines: `Request:`, `Why:`, `Expected:`, `Impact:`.
- Run static verification check:
  ```bash
  sh .verify/checks/feature_coverage.sh .verify/features/<feature>.md .verify/functional/<feature>.hurl
  ```
  Ensure zero unmapped or duplicate entries.

### Step 5: Execute & Measure Coverage
- Run through the allowlist:
  ```bash
  sh .verify/run.sh <feature>
  ```
- Verify that tests pass cleanly and log artifacts survive in `.verify/artifacts/`.
