---
feature: cli-proof
knowledge_base: "[[../knowledge-base/cli-proof]]"
related:
  - "[[core-crate]]"
  - "[[ci-pipeline]]"
---

# cli-proof

Summary: The tekmor command line as a proof eval. It runs allowlisted checks and records their outcomes, reports a receipt for the current code, and runs the gates that stop an agent from skipping them.

## Sub-features

- `verify-run`: Runs an allowlisted command and records its outcome.
  - Intent: Only a real check run on the current code counts as proof.
  - Behaviour: `tekmor verify -- <cmd>` runs a command from `.verify/verify.toml` and stores exit code, command, source, code hash and head SHA (`verify.rs`).
  - Outcome: The recorded run is what the receipt and the stop gate read.
- `receipt`: Shows whether the current commit and code passed.
  - Behaviour: `tekmor verify receipt [--markdown|--json]` builds a receipt from recorded runs for the current head and code hash (`receipt.rs`). Exit code is 0 only when the verdict is Passed.
  - Outcome: One verdict, used by the CLI, the stop gate and CI.
- `check-summary`: Per check, how often it failed.
  - Behaviour: `tekmor verify summary [--json]` groups recent runs by command, failing checks first (`tekmor-core` `evidence::summarize`).
  - Outcome: Shows which checks fail most.
- `stop-gate`: Blocks a turn from ending while changed code has no passing run or no feature map.
  - Behaviour: `tekmor stop-gate` (`stop_gate.rs`), registered as the Claude Code Stop hook.
  - Outcome: The block names the repo and the missing feature map or check.
- `bash-gate`: Blocks or warns on risky shell commands before they run.
  - Behaviour: `tekmor bash-gate` (`command_policy.rs`, `command_rules.rs`). It also refuses commits that skip git hooks (`commit_gate.rs`).
  - Outcome: Registered as the Claude Code PreToolUse hook for Bash.
- `feature-map-gap`: Names the feature map that owns changed code.
  - Behaviour: `feature_map.rs` reads `.verify/features` and the `## Code` section of each map.
  - Outcome: A stop-gate block points at the map to update.
- `gate-events`: Appends gate decisions to a log.
  - Behaviour: `gate_events.rs` writes `gate-events.jsonl` under the logs directory.
  - Outcome: Gate decisions can be reviewed after the fact.
- `claude-gate-hooks`: Registers the gates in Claude Code settings.
  - Behaviour: `tekmor install [--global]` writes a PreToolUse Bash entry (`bash-gate`) and a Stop entry (`stop-gate`) into the settings file (`host_hooks.rs`).
  - Outcome: Only the two proof gates are registered.
- `extensions`: Installs and runs optional extension binaries.
  - Behaviour: `tekmor ext list|add|remove` (`extensions.rs`, `registry_sync.rs`). Extensions are kept for later use; none are bundled.
  - Outcome: The extension mechanism is present without any bundled extension.

## Code

- `tekmor/src/verify.rs`
- `tekmor/src/receipt.rs`
- `tekmor/src/evidence_reader.rs`
- `tekmor/src/stop_gate.rs`
- `tekmor/src/command_policy.rs`
- `tekmor/src/command_rules.rs`
- `tekmor/src/commit_gate.rs`
- `tekmor/src/feature_map.rs`
- `tekmor/src/gate_events.rs`
- `tekmor/src/spec.rs`
- `tekmor/src/project_key.rs`
- `tekmor/src/host_hooks.rs`
- `tekmor/src/extensions.rs`
- `tekmor/src/registry_sync.rs`
- `tekmor/src/settings.rs`
- `tekmor/src/config.rs`
- `tekmor/src/global_workspace.rs`
- `tekmor/src/db/store.rs`
- `tekmor/src/db/migrations.rs`
- `tekmor/src/main.rs`
- `tekmor/src/cli_help.rs`
- `tekmor/src/lib.rs`

## How to get to it (user POV)

- `tekmor verify -- .verify/run.sh`, then `tekmor verify receipt`.
- `tekmor install` to register the gates in Claude Code.

## Verification Suites (Targeted Test Commands)

- **Unit and integration tests:** `cargo test -p tekmor -- --test-threads=1`
- **Static check:** `cargo check -p tekmor`
- **Allowlisted run:** `.verify/run.sh`

## Gotchas

- The shared store keeps only verify outcomes and meta. Older tables may still exist in an existing `brain.db`; they are no longer read.
