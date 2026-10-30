# agent-brain feature maps

Each feature map is a `<feature>.md` file. It has frontmatter (`feature`, `knowledge_base`, and `related` wikilinks), an H1 title, a one-paragraph summary, and four contract H2 sections: Sub-features, How to get to it, Verification Suites, and Gotchas. A `## Code` section lists the paths the feature owns, which the stop gate requires. Run a feature's unit checks with its Verification Suites command, and run the whole harness with `.verify/run.sh`. Print the graph of feature maps with `agent-brain feature-graph`.

Every command in this index and in the maps is on the allowlist in `.verify/verify.toml`.

| Feature | Map | Knowledge base | Unit check | Static check |
|---------|-----|----------------|------------|--------------|
| Core routing crate | [core-crate.md](core-crate.md) | [core-crate](../knowledge-base/core-crate.md) | `cargo test -p agent-brain-core` | `.verify/checks/core_boundary.sh` |
| CLI, verify recorder, memory, install | [cli-verify-and-memory.md](cli-verify-and-memory.md) | [cli-verify-and-memory](../knowledge-base/cli-verify-and-memory.md) | `cargo test --all-features -- --skip graphify_1k_ci_gate --skip scale_1k_passes_gate --test-threads=1` | `.verify/checks/core_boundary.sh` |
| Skill packages (install, update, remove, index scope) | [packages.md](packages.md) | [packages](../knowledge-base/packages.md) | `cargo test --all-features -- --skip graphify_1k_ci_gate --skip scale_1k_passes_gate --test-threads=1` | `.verify/run.sh` |
| Shared daemon, stdio proxy, embedder idle unload | [serve-daemon-and-proxy.md](serve-daemon-and-proxy.md) | [serve-daemon-and-proxy](../knowledge-base/serve-daemon-and-proxy.md) | `cargo test --all-features embed::tests::` and `cargo test --all-features --test proxy_lifecycle` | `sh .verify/checks/require_tests.sh --all-features -- --test-threads=1` |
