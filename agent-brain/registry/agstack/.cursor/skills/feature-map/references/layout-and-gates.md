# Layout, gates, and review

Load this file when you set up `.verify/` in a repository, or when you run or change the quality gates.

## Layout

```text
.verify/
├── verify.toml        allowlisted commands
├── run.sh             phases: doctor, unit, static, functional; argument: feature or all
├── doctor.sh          read-only checks: tools, ports, token presence (the token value is never printed)
├── gates.conf         ordered quality gates for this repository
├── high-risk.txt      path patterns that need an independent review
├── features/
│   ├── README.md      preconditions, driving conventions, proof standards, feature index
│   └── <feature>.md   contracts (see contract-format.md)
├── functional/
│   ├── <feature>.hurl
│   └── payloads/      rendered into artifacts/<RUN_ID>/ by run.sh
├── checks/            generic harness checks and gate scripts
├── suites/            repository-specific checks and helpers
├── env/               test variables; each live value records its source
└── artifacts/         gitignored; logs, reports, and evidence records
```

## Entry coverage

`checks/feature_coverage.sh <feature.md> <functional.hurl>` fails when a request entry is covered by no step or by two. Call it from the static phase of `run.sh`.

## Coverage floor

`checks/coverage_gate.sh <jacoco.xml> <line-floor> <branch-floor>` reads the report-level counters from a JaCoCo XML report and fails when either is below its floor. Use the repository's own report path. The floor is the policy target, or a stricter value the repository sets.

The number is only as good as the test run that produced it. A targeted test run measures only the classes it exercises. Measure the module with the full suite before claiming a module's coverage.

## Quality gates

`checks/gate_report.sh <gates.conf> <out-dir>` runs each gate in the file's order. Each line is `name|command`, or `name|NOT-AVAILABLE: reason`.

- The first failing gate stops the run. Later gates are recorded as not run.
- A gate that cannot run in the environment is recorded as not available, with the reason. It does not pass.
- The report is written to `<out-dir>/evidence.md`. It lists the gate table, the changed files, the high-risk matches, the review status, the gates not available, and the result.
- The report states that it does not establish that the code is bug-free.

Order gates from cheapest to most expensive: contract coverage, format, lint, build, unit tests, coverage, static analysis, functional tests.

Use the repository's own tools for each gate. Read the repository's build configuration, its CODEOWNERS file, and its pull request template before you choose them. Typical requirements:

| Gate | Requirement |
|---|---|
| Format | The repository's formatter runs on changed files, with no unrelated churn |
| Lint | The repository's configured static checker reports no new findings |
| Build | A clean build passes |
| Unit tests | Changed behaviour is covered, and each fixed defect has a regression test |
| Coverage | Measured with the repository's report tooling against the floor |
| Static analysis | The repository's analysis service is run when reachable; otherwise the gate is not available |
| Release notes | When source changes, the repository's release-notes location has a changed entry |
| Functional tests | The Hurl suites run against the local services |

## High-risk changes and independent review

`high-risk.txt` lists path patterns, one per line, in shell glob form. A change touching a matching path needs an independent review.

The review record is `<out-dir>/review.txt`, with these lines:

```text
author: <name of the change author>
reviewer: <handle of the reviewer>
problem: <one line>
scope: <one line>
tests: <numbered test scenarios, one line>
```

The gate fails when:

- the reviewer is missing or equals the author;
- the reviewer is not listed in the repository's CODEOWNERS file, when that file exists;
- a required field is missing or empty.

The fields follow the repository's pull request template, so the same review content serves both the record and the pull request.
