# tekmor feature maps

Each feature map is a `<feature>.md` file with frontmatter (`feature`, `knowledge_base`, `related`), sub-features, a `## Code` section that lists the owned paths, and verification suites.

Every command in the maps is on the allowlist in `.verify/verify.toml`.

| Feature | Map | Knowledge base | Unit check | Static check |
|---------|-----|----------------|------------|--------------|
| Proof CLI: verify, receipt, gates, extensions | [cli-proof.md](cli-proof.md) | [cli-proof](../knowledge-base/cli-proof.md) | `cargo test -p tekmor -- --test-threads=1` | `cargo check -p tekmor` |
| Core proof types and gate rule | [core-crate.md](core-crate.md) | [core-crate](../knowledge-base/core-crate.md) | `cargo test -p tekmor-core` | `.verify/checks/core_boundary.sh` |
| GitHub Actions CI and release | [ci-pipeline.md](ci-pipeline.md) | [ci-pipeline](../knowledge-base/ci-pipeline.md) | see the map | see the map |
| Install, release, migration scripts, Makefile, Containerfile | [repo-tooling.md](repo-tooling.md) | [repo-tooling](../knowledge-base/repo-tooling.md) | see the map | see the map |
