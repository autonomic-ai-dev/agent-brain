# agent-brain feature maps

Each feature map is a `<feature>.md` file with four H2 sections: Sub-features, How to get to it, Verification Suites, and Gotchas. Run a feature's unit checks with its Verification Suites command, and run the whole harness with `.verify/run.sh`.

| Feature | Map | Unit check | Static check |
|---------|-----|------------|--------------|
| Core routing crate | [core-crate.md](core-crate.md) | `cargo test -p agent-brain-core` | `.verify/checks/core_boundary.sh` |
| CLI, verify recorder, memory, install | [cli-verify-and-memory.md](cli-verify-and-memory.md) | `cargo test --all-features` (whole crate; `cargo test -p agent-brain-core` for the core only) | `sh .verify/checks/core_boundary.sh` |
| Skill packages (install, update, remove, index scope) | [packages.md](packages.md) | `cargo test --all-features` | `.verify/run.sh` |
| Shared daemon, stdio proxy, embedder idle unload | [serve-daemon-and-proxy.md](serve-daemon-and-proxy.md) | `cargo test --all-features embed::tests::` and `cargo test --all-features --test proxy_lifecycle` | `sh .verify/checks/require_tests.sh --all-features` |
