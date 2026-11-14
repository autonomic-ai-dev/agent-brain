---
feature: ci-pipeline
knowledge_base: "[[../knowledge-base/ci-pipeline]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[bench-extension]]"
---

# ci-pipeline

Summary: GitHub Actions for agent-brain. One CI workflow runs clippy, the Rust tests and the hook tests on every push and pull request to master. A release on a `v*` tag runs that same CI workflow, then builds the signed macOS binary and publishes the GitHub release.

## Sub-features

- `ci-check`: One job that gates merges to master.
  - Intent: Give a single, plain place to see whether a change builds, lints and passes its tests.
  - Behaviour: `.github/workflows/ci.yml` runs `cargo clippy` with warnings denied, `cargo test --release -p agent-brain --features extras -- --test-threads=1`, and `python3 agent-brain/hooks/test_route_gate.py`. It runs on push and pull request to master, and it is reusable by the release workflow.
  - Outcome: A failing lint, test or hook test fails the workflow.
- `release-publish`: Tag-driven release.
  - Intent: Ship a signed binary and release notes from a `v*` tag without a second pipeline.
  - Behaviour: `.github/workflows/release.yml` runs the CI check, then `.github/workflows/reusable-build.yml`, then publishes a GitHub release from `dist/*` with notes from `.github/scripts/changelog-extract.sh`.
  - Outcome: Each tag produces one release with the build artifacts attached.

## Code

- `.github/workflows`
- `.github/pipelines`

## How to get to it (user POV)

- Open a pull request against master. The CI check runs and must pass before merge.
- Push a tag such as `v1.0.0`. The release runs the check, builds the binary, and publishes it.
- Manual runs of the CI check are available from the Actions tab (`workflow_dispatch`).

## Verification Suites (Targeted Test Commands)

- **Local equivalent:** `.verify/run.sh` runs the clippy and test steps that the CI check runs.
- **Preconditions:** Rust toolchain and the repository's locked dependencies. The workflow itself runs only on GitHub Actions; it cannot be executed locally.

## Gotchas

- The protected-path workflow is removed. Changes to tests, `.verify/`, migrations and security config are reviewed in pull request review, not gated by a CI label.
- The pipeline-compose wrapper and the stage workflows are removed. Only `ci.yml`, `release.yml` and `reusable-build.yml` remain.
- `reusable-build.yml` has two shellcheck warnings (SC2193, SC2046) that predate this change.
