---
feature: "[[../features/graphify-extension]]"
---

# Graphify extension

## What it does

The `graphify` command builds and queries a codebase graph. It moved out of the core binary into the `agent-brain-graphify` extension, the same pattern as the bench extension.

## How it works

1. `agent-brain graphify <sub>` routes to the installed extension (`agent-brain ext add graphify`).
2. The extension (`extensions/graphify/src/main.rs`) loads the config, opens the engine, and calls the core `agent_brain::graphify::run_cli`.
3. The graph engine, the job queue, and the MCP tools stay in the core library (`agent-brain/src/graphify/`).

## Where the code is

- Extension: `extensions/graphify/src/main.rs`, `extensions/graphify/Cargo.toml`
- Core graph engine (unchanged): `agent-brain/src/graphify/`
- Core dispatch removed: `agent-brain/src/main.rs` (the `graphify` arm and the extras name list)

## How it is verified

- `cargo build -p agent-brain-graphify`
- `./target/debug/agent-brain-graphify graphify status` (prints `[]` with no repo enabled)
- `./target/debug/agent-brain-graphify bogus` prints the usage line

## Known limits

- The core graph engine still ships in the core library. A later slice can move the MCP graph tools and route code context into the extension.
- The extension depends on the core library at the same version, so it must be rebuilt with the core.

## Related

- Feature map: [[../features/graphify-extension]]
- Extension mechanism and install trust: [[cli-verify-and-memory]]
- Previous slice: [[bench-extension]]
