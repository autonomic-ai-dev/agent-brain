---
feature: extras-extension
knowledge_base: "[[../knowledge-base/extras-extension]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[graphify-extension]]"
  - "[[bench-extension]]"
---

# extras-extension

Summary: The dataset export, weekly digest, architecture distill, and gRPC bridge commands, shipped as the optional `agent-brain-extras` extension. The fixture and skills.sh commands moved to the bench extension (see [[bench-extension]]). The core binary and core library no longer contain these commands.

## Sub-features

- `extras-dataset-distill`: Dataset export and stats, spine ingest, and architecture distill.
  - Behaviour: `agent-brain dataset export|stats|ingest-spine` and `agent-brain distill [clusters]`. The extension runs them against the core store.
  - Outcome: The same output as before the move. A missing subcommand prints the usage line.
- `extras-digest`: Weekly operator digest.
  - Behaviour: `agent-brain digest --weekly`. A missing `--weekly` prints the usage line.
  - Outcome: The weekly digest of routing activity.
- `extras-grpc-serve`: The orchestrator gRPC bridge.
  - Behaviour: `agent-brain grpc serve [--addr HOST:PORT]`. The extension loads the config, runs the bootstrap, and serves the routing service. The default address is `127.0.0.1:7842`.
  - Outcome: A gRPC server on the address. Any other `grpc` subcommand prints the usage line.

## Code

- `extensions/extras`
- `extensions/extras/proto/agent_brain/v1/routing.proto` (the gRPC service definition)
- `extensions/extras/build.rs` (generates the gRPC code)
- `Cargo.toml`
- `agent-brain/build.rs`
- `agent-brain/proto/agent_brain/v1/routing.proto`

## How to get to it (user POV)

- `agent-brain ext add extras`, then `agent-brain dataset stats` or `agent-brain grpc serve`
- Developers can run `./target/debug/agent-brain-extras dataset stats` after `cargo build -p agent-brain-extras`.

## Verification Suites (Targeted Test Commands)

- **Build:** `cargo build -p agent-brain-extras`
- **Unit tests:** `cargo test -p agent-brain-extras` (the spine-ingest tests, the digest test, and the gRPC bridge test)
- **Smoke:** `./target/debug/agent-brain-extras dataset stats` prints JSON and exits 0; `./target/debug/agent-brain-extras nope` prints the usage line.
- **Allowlisted:** `.verify/verify.toml` lists `cargo test -p agent-brain-extras`, so these tests run in the verify gate.

## Gotchas

- `agent-brain/build.rs` and `agent-brain/proto/agent_brain/v1/routing.proto` are listed in Code because they were deleted from core. The gRPC generation and proto now live under `extensions/extras`.
- The gRPC code is generated at build time from `proto/agent_brain/v1/routing.proto` by this extension's `build.rs`. The core crate no longer generates it, and no longer depends on tonic or prost.
- The extension is a binary crate, so its unit tests live in its modules. It has no library target for integration tests.
- The extension depends on the core library at the same version, so it must be rebuilt with the core.
