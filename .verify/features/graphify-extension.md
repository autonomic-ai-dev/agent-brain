---
feature: graphify-extension
knowledge_base: "[[../knowledge-base/graphify-extension]]"
related:
  - "[[cli-verify-and-memory]]"
  - "[[bench-extension]]"
---

# graphify-extension

Summary: The codebase graph command (`graphify enable | disable | status | ingest | run | query`), shipped as the optional `agent-brain-graphify` extension instead of inside the core binary. The graph engine itself still lives in the core library, so the MCP graph tools and route code context are unchanged.

## Sub-features

- `graphify-command`: Runs the codebase graph commands from the extension binary.
  - Behaviour: `agent-brain graphify <sub> [--repo PATH] [--trigger T] [--mode M] [--id ID] [--budget N]`, where `query` takes its question as the next argument or `--question`. The extension loads the config, opens the engine, and calls the core `graphify::run_cli`.
  - Outcome: The same output as before the move. An unknown subcommand prints the usage line and exits nonzero.

## Code

- `extensions/graphify`
- `Cargo.toml`

## How to get to it (user POV)

- `agent-brain ext add graphify`, then `agent-brain graphify status`
- Developers can run `./target/debug/agent-brain-graphify graphify status` after `cargo build -p agent-brain-graphify`.

## Verification Suites (Targeted Test Commands)

- **Build:** `cargo build -p agent-brain-graphify`
- **Smoke:** `./target/debug/agent-brain-graphify graphify status` prints a JSON list and exits 0.
- **Allowlisted:** `.verify/run.sh`, which runs the workspace suite including this crate.

## Gotchas

- The extension links the whole core library with `extras`, so its binary is about as large as the core binary.
- The core binary no longer has a `graphify` command. `agent-brain graphify` routes to the installed extension, and without it the command is unknown.
- The MCP tools `query_codebase`, `graphify_job_status`, and the job enqueue path, plus route code context, still run in the core binary. Moving them is a later slice.
