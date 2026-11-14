---
feature: "[[../features/ci-pipeline]]"
---

# CI pipeline

## What it does

GitHub Actions builds, lints and tests tekmor, and publishes releases from `v*` tags.

## How it works

1. **CI.** `.github/workflows/ci.yml` runs on push and pull request to master, and on manual dispatch. It has one job: clippy with warnings denied, the Rust tests with `--features extras` and one test thread, and the hook tests.
2. **Release.** `.github/workflows/release.yml` runs on `v*` tags. It calls the CI workflow, then `.github/workflows/reusable-build.yml` (the signed macOS build), then publishes the release.
3. **Removed.** The compose wrapper (`.github/pipelines/`), the stage workflows, the path filter on CI, and the protected-path label check.

## Decisions

- One CI job instead of a staged pipeline. The stages existed only to pass a version and SHA between jobs, which the release now gets from the same run.
- No path filter. Every push and pull request runs the full check, so a skipped check cannot be mistaken for a passing one.
- No protected-path label check. The label gate blocked ordinary changes to tests and `.verify/`; review in the pull request replaces it.
