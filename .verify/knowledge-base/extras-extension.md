---
feature: "[[../features/extras-extension]]"
---

# Extras extension

## What it does

Dataset export, the weekly digest, architecture distill, and the gRPC bridge are shipped in the optional `tekmor-extras` extension. The fixture builder and the skills.sh snapshot moved to the bench extension, next to the eval gates they feed.

## How it works

1. `tekmor <cmd>` routes to the installed extension when the command is `dataset`, `digest`, `distill`, or `grpc`.
2. `extensions/extras/src/main.rs` dispatches each command:
   - `dataset` and `distill` run the modules in `extensions/extras/src/` (`dataset.rs`, `distill.rs`, `spine_ingest.rs`).
   - `digest` runs `operator_digest.rs`, which reads the store.
   - `grpc serve` runs the gRPC server in `extensions/extras/src/grpc/`. Its code is generated from `proto/tekmor/v1/routing.proto` by `build.rs`.
3. The core library provides the shared pieces: the store, the engine, the config, and the route types.

## Where the code is

- Extension entry point: `extensions/extras/src/main.rs`
- Dataset, distill, spine ingest: `extensions/extras/src/dataset.rs`, `distill.rs`, `spine_ingest.rs`
- Weekly digest: `extensions/extras/src/operator_digest.rs`
- gRPC bridge: `extensions/extras/src/grpc/` and `extensions/extras/proto/`
- Build: `extensions/extras/build.rs` and `Cargo.toml`
- Route handler: `extensions/extras/src/grpc/server.rs` builds `tekmor::engine::RouteRequest` from the proto request. The core's `route_task` takes that struct, not positional arguments.

## How it is verified

- `cargo build -p tekmor-extras`
- `cargo test -p tekmor-extras` (spine ingest, digest, and the gRPC bridge fields)
- `./target/debug/tekmor-extras dataset stats` (read-only, prints JSON)
- `./target/debug/tekmor-extras grpc status` prints the usage line

## Known limits

- `dataset.rs` and `distill.rs` needed `bytes_to_f32` made public in core (`tekmor/src/db/store.rs`).
- The moved `spine_ingest.rs` had two functions with no callers, `merge_jsonl_files` and its helper `md5_hash`. They were removed rather than suppressed.
- The core binary no longer has tonic, prost, or a build script, so it builds faster and without protobuf tooling.

## Related

- Feature map: [[../features/extras-extension]]
- Extension mechanism: [[cli-verify-and-memory]]
- Earlier slices: [[graphify-extension]], [[bench-extension]]
