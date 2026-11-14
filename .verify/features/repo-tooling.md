---
feature: repo-tooling
knowledge_base: "[[../knowledge-base/repo-tooling]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[ci-pipeline]]"
---

# repo-tooling

Summary: The scripts, Makefile and Containerfile that install, build, sign, release and migrate tekmor on a developer machine, plus the repository's ignore rules.

## Sub-features

- `install-script`: Installs the tekmor binary from a GitHub release.
  - Behaviour: `scripts/install.sh` downloads the release asset for the platform into `~/.local/bin`, and reads the repository from `TEKMOR_REPO`.
  - Outcome: `tekmor` is on the user's PATH after the script runs.
- `release-build-sign`: Builds, signs and syncs a release binary.
  - Behaviour: `scripts/build-release-macos.sh` builds the release binary, `scripts/sign-macos.sh` signs it, and `scripts/sync-local-release.sh` copies it to the local install.
  - Outcome: A signed macOS binary ready for the release workflow.
- `migrate-to-tekmor`: Moves a machine from the agent-brain name to tekmor.
  - Behaviour: `scripts/migrate-to-tekmor.sh` copies the legacy data home, renames the old rule files, rewrites names in host configs, the shell files and `~/.autonomic/memory/config.yaml` (where tekmor reads its settings) with a backup for each edited file, and reinstalls host hooks. `--dry-run` only prints the changes.
  - Outcome: Each machine runs under the tekmor name. See `tekmor/docs/migrate-to-tekmor.md`.
- `product-init`: Creates a product scaffold from the repository's templates.
  - Behaviour: `scripts/product-init.sh`.
- `container-and-make`: Local build targets and a container image.
  - Behaviour: `Makefile` lists the build, test and install targets. `Containerfile` builds the image.

## Code

- `.gitignore`
- `Containerfile`
- `Makefile`
- `scripts`

## How to get to it (user POV)

- Install: `scripts/install.sh`
- Migrate an existing machine: `scripts/migrate-to-tekmor.sh --dry-run`, then `scripts/migrate-to-tekmor.sh`
- Build a release on macOS: `scripts/build-release-macos.sh`

## Verification Suites (Targeted Test Commands)

- **Allowlisted Verification:** `.verify/run.sh`
- **Preconditions:** Shell and the repository's Rust toolchain. The release and sign scripts also need macOS and the signing identity; they cannot run on Linux.

## Gotchas

- `scripts/install.sh` and the release assets use the tekmor names. They work only after the first tekmor release is published.
- The migration script runs on each machine. Run it once per machine, after the tekmor binary is on the PATH.
