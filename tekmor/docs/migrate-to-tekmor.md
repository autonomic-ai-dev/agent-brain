# Migrating from agent-brain to tekmor

agent-brain is now called tekmor. Each machine that ran agent-brain needs one migration. The script `scripts/migrate-to-tekmor.sh` does it.

## What changes

| Before | After |
|---|---|
| binary `agent-brain` (`~/.local/bin/agent-brain`) | binary `tekmor` (`~/.local/bin/tekmor`) |
| MCP server `agent-brain` and tools `mcp__agent-brain__*` | MCP server `tekmor` and tools `mcp__tekmor__*` |
| env vars `AGENT_BRAIN_*` | `TEKMOR_*` |
| legacy data home `~/.agent_brain` | `~/.tekmor` (copied, original kept) |
| rules `agent-brain-route.md`, `agent-brain.mdc` | `tekmor-route.md`, `tekmor.mdc` |
| host hooks and marker blocks `agent-brain:begin` | `tekmor:begin` |

Unchanged: the shared memory store under `~/.autonomic/memory`, and the GitHub repository `autonomic-ai-dev/agent-brain` until it is renamed on GitHub.

## Steps on each machine

1. Install the tekmor binary. Use the release from `scripts/install.sh`, or build it with `cargo build --release -p tekmor` and copy `target/release/tekmor` to `~/.local/bin/tekmor`.
2. Check that `tekmor` is on PATH: `command -v tekmor`.
3. Preview the change: `scripts/migrate-to-tekmor.sh --dry-run`. It prints each file it would rewrite or rename and writes nothing.
4. Apply it: `scripts/migrate-to-tekmor.sh`. Every edited file keeps a backup named `<file>.pre-tekmor-<timestamp>`.
5. Restart every open host session (Claude Code, Cursor, Codex, Gemini, OpenCode) and stop any running `serve` process, so the new MCP server name is picked up.
6. Check the result: `tekmor doctor`.
7. When tekmor works, remove the old binary: `rm ~/.local/bin/agent-brain`. Remove the old data home `~/.agent_brain` only after you have checked `~/.tekmor`.

## If something goes wrong

- Restore a file from its backup: `mv <file>.pre-tekmor-<timestamp> <file>`.
- The script stops at the first failing step. Fix the cause and rerun it. A second run changes nothing that is already migrated.
