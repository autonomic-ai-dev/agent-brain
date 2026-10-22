# Authoring rules and discovery

Load this file before you create or change a feature map.

## Discovery

Build the map by reading the repository, never by asking the user for something the repository shows.

1. **How the application starts:** read the build manifest, package scripts, container manifests, and the entry point. Note the ports, environment defaults, and readiness signals.
2. **Which surfaces exist:** read the routing tables, command-line parsers, service definitions, and message-topic subscribers.
3. **Which tests exist:** find the existing test files and harness scripts, and link them into the contract.
4. **Which data is required:** read the migrations, seed scripts, and fixture generators to find the baseline preconditions.

Ask the user only for a secret or credential that is outside the repository and cannot be generated or mocked locally.

## Authoring rules

These rules apply to every feature map change:

1. **Read the code, not the user.** Trace routes, handlers, and tests to build the contract.
2. **No placeholders in commands.** Commands use literal values or shell variables such as `"$RUN_ID"` or `"$AUTH_TOKEN"`. Angle-bracket placeholders are not allowed in commands or in template names.
3. **User-observable behaviour only.** Private helpers, internal fields, and internal comments do not belong in the map. Map external interfaces and what callers observe.
4. **Never weaken an assertion to pass.** When a test fails, fix the code, or record the root cause under Gotchas. Do not remove sub-features or loosen assertions to get a green run.
5. **Index every feature.** Each `features/<feature>.md` is listed in `features/README.md`. No orphan contracts.
6. **Pair API features with functional tests.** A feature with an HTTP or RPC surface has a `functional/<feature>.hurl` file.
7. **Four-part step records and entry coverage.** Follow `contract-format.md`. Run the coverage script from the static phase.
8. **Sourced expectations.** Values that depend on the environment's reference data (calendar dates, period or cycle identifiers, derived amounts) come from a recorded live query. Write the query, its result, and its date in the contract. Values derived from code name the class or method. Values not confirmed by a live run are marked `[UNCONFIRMED]`. A live value that contradicts a local config file wins; record the change in Gotchas.
9. **Boundary fixtures match their step.** A payload used by a boundary step contains the boundary value the step names. Check this before accepting the contract.
10. **Unique request identifiers per run.** Within one run, every request identifier is unique across all feature files. State the suffix scheme in `features/README.md`.
11. **Name coverage gaps.** Gotchas lists what the suite does not test.
12. **Concatenation caution.** If test files were merged, Gotchas states that captured values now cross former file boundaries.
13. **Link the knowledge base.** When the repository keeps a knowledge base (entity folders with a guide and an issues file, plus an index), the entity's guide lists each feature contract that covers it, and the entity has an index row. The guide points to the contract. The contract does not copy the guide's routing tables.

## Index specification: `features/README.md`

The index holds:

1. **Baseline preconditions:** startup variables, data paths, and the doctor checks.
2. **Driving conventions:** output flags, timeouts, and cleanup rules.
3. **Proof standards:** where logs, transcripts, and responses are kept (`.verify/artifacts/`).
4. **Feature index:** a table with one row per feature file, giving its sub-feature count and its primary surface.
