---
feature: packages
knowledge_base: "[[../knowledge-base/packages]]"
related:
  - "[[cli-verify-and-memory]]"
---

# packages

Summary: Installs skill packages from GitHub into the brain home, records each one in `packages.json`, and indexes their skills so the router can find them.

## Sub-features

- `package-add-clone`: Clones a `owner/repo` source into its own directory.
  - Intent: Keep two repositories with the same name from sharing one checkout.
  - Behaviour: `tekmor package add <owner/repo>` clones into `packages/<owner>-<repo>`, lowercased. Re-adding an installed source pulls it instead of cloning.
  - Outcome: `package list` shows the name, source, commit, and path. `slavingia/skills` installs as `slavingia-skills`.
- `package-remove-purge`: Removes a package and its indexed items.
  - Behaviour: `tekmor package remove <name>` deletes the registry entry, purges the indexed items under its install path, and deletes the directory.
  - Outcome: The package is gone from `package list` and its skills no longer match `skill_search`.
- `package-update`: Pulls installed packages to their latest ref.
  - Behaviour: `tekmor package update [name]` skips bundles and any directory that is not a git checkout.
  - Outcome: The recorded commit changes to the pulled head.
- `package-index-scope`: Tags indexed items with the package they came from.
  - Behaviour: The first path component under `packages/` becomes the item's package key, so the directory name is also the scope.
  - Outcome: Items from `slavingia-skills` carry that scope.

## Code

- `tekmor/src/packages/`
  - Owns: package registry (`packages.json`), clone and update, curated aliases, bundles, and the `package_name` owner-repo naming.
- `tekmor/registry`
  - Owns: the bundled registry manifests (`manifest.json`, `packages.json`, `utilities.json`, `workflows.json`), workflow definitions, and the product and supervisor rule packs under it.

## How to get to it (user POV)

- CLI: `tekmor package add <owner/repo>`, `package list`, `package update`, `package remove <name>`
- MCP: `skill_search` and `skill_get` return skills from installed packages.

## Verification Suites (Targeted Test Commands)

- **Unit Tests:** `cargo test --all-features -- --skip graphify_1k_ci_gate --skip scale_1k_passes_gate --test-threads=1` (whole crate; the `packages::` tests are included)
- **Local-repository tests:** clone, update, registry, removal and `update_packages` run against `file://` git repositories in `packages::tests`, so they need no network.
- **Targeted package test:** `cargo test --all-features packages::tests::package_name_includes_owner_so_same_repo_names_differ`
- **Allowlisted Verification:** `.verify/run.sh`, listed in `.verify/verify.toml`
- **Preconditions:** Rust toolchain and network access to GitHub for `package add`. Run install checks against a temporary `HOME`, not the user's `~/.autonomic`.

### Verification Steps

1. **package-name-unit** (sub-feature: package-add-clone)
   - Request: `cargo test --all-features packages::tests::package_name_includes_owner_so_same_repo_names_differ`
   - Why: Proves the owner prefix separates two repositories with the same name.
   - Expected: 1 passed, 0 failed.
   - Impact: A failure means `slavingia/skills` and `mattpocock/skills` collide again.

## Gotchas

- Installs made before the owner prefix keep their old names (for example `skills`). Remove and re-add them to get the new name.
- The Homebrew binary at `/opt/homebrew/bin/tekmor` must be rebuilt or reinstalled before `package add` uses the owner-prefixed name globally.
- `package remove` deletes the checkout directory. Re-add to restore it.

- Formatting pass (rustfmt, 2026-11-07): line breaks and spacing only, no behaviour change.
