---
feature: extras-extension
knowledge_base: "[[../knowledge-base/extras-extension]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[graphify-extension]]"
  - "[[bench-extension]]"
---

# extras-extension

Summary: The dataset export, weekly digest, architecture distill, and gRPC bridge commands, shipped as the optional `tekmor-extras` extension. The fixture and skills.sh commands moved to the bench extension (see [[bench-extension]]). The core binary and core library no longer contain these commands.

## Sub-features

- `extras-dataset-distill`: Dataset export and stats, spine ingest, and architecture distill.
  - Behaviour: `tekmor dataset export|stats|ingest-spine` and `tekmor distill [clusters]`. The extension runs them against the core store.
  - Outcome: The same output as before the move. A missing subcommand prints the usage line.
- `extras-digest`: Weekly operator digest.
  - Behaviour: `tekmor digest --weekly`. A missing `--weekly` prints the usage line.
  - Outcome: The weekly digest of routing activity.
- `extras-grpc-serve`: The orchestrator gRPC bridge.
  - Behaviour: `tekmor grpc serve [--addr HOST:PORT]`. The extension loads the config, runs the bootstrap, and serves the routing service. The default address is `127.0.0.1:7842`.
  - Outcome: A gRPC server on the address. Any other `grpc` subcommand prints the usage line.

- `extras-digest-measured`: The weekly digest reports measured routing figures only.
  - Behaviour: `extensions/extras/src/operator_digest.rs` drops the estimated savings fields (`routes_with_savings`, `avg_saved_pct`, `total_saved_tokens`). It reports average routed tokens across `route_calls`.
  - Outcome: The digest shows no estimated savings.

## Code

- `extensions/extras`
- `extensions/extras/proto/tekmor/v1/routing.proto` (the gRPC service definition)
- `extensions/extras/build.rs` (generates the gRPC code)
- `Cargo.toml`
- `tekmor/build.rs`
- `tekmor/proto/tekmor/v1/routing.proto`

## How to get to it (user POV)

- `tekmor ext add extras`, then `tekmor dataset stats` or `tekmor grpc serve`
- Developers can run `./target/debug/tekmor-extras dataset stats` after `cargo build -p tekmor-extras`.

## Verification Suites (Targeted Test Commands)

- **Build:** `cargo build -p tekmor-extras`
- **Unit tests:** `cargo test -p tekmor-extras` (the spine-ingest tests, the digest test, and the gRPC bridge test)
- **Smoke:** `./target/debug/tekmor-extras dataset stats` prints JSON and exits 0; `./target/debug/tekmor-extras nope` prints the usage line.
- **Allowlisted:** `.verify/verify.toml` lists `cargo test -p tekmor-extras`, so these tests run in the verify gate.

## Gotchas

- `tekmor/build.rs` and `tekmor/proto/tekmor/v1/routing.proto` are listed in Code because they were deleted from core. The gRPC generation and proto now live under `extensions/extras`.
- The gRPC code is generated at build time from `proto/tekmor/v1/routing.proto` by this extension's `build.rs`. The core crate no longer generates it, and no longer depends on tonic or prost.
- The extension is a binary crate, so its unit tests live in its modules. It has no library target for integration tests.
- The extension depends on the core library at the same version, so it must be rebuilt with the core.
- The gRPC `route_task` handler (`src/grpc/server.rs`) maps the request fields into `tekmor::engine::RouteRequest` before calling `route_task`. Field names in `routing.proto` must match that struct when either changes.
