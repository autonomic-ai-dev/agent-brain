# packages knowledge base

## Owned code

- `agent-brain/src/packages/`: package registry, clone, update, remove, and curated aliases.

## Behaviour that matters

- Clone directory and package name are `owner-repo` (lowercased). Before this change they were the repo name alone, so `slavingia/skills` and `mattpocock/skills` both became `skills` and shared one directory.
- `add_package` reuses an existing install directory by pulling it, without checking its origin. A stale checkout under the same name is therefore kept and recorded under the new source. Remove the stale directory before re-adding.
- Indexed items take their package scope from the first path component under `packages/`, so a rename changes the scope of new items.

## Verification

- `cargo test --all-features` covers the `packages::` unit tests, including `package_name_includes_owner_so_same_repo_names_differ`.
- Not covered by tests: the real `package add` network clone and the stale-directory reuse path.
