# agent-brain

> **Local-First Cognitive Governor, Turn Router & Procedure Graph for Coding Agents.**

[![Version](https://img.shields.io/badge/version-1.0.0--rc7-blue.svg)](CHANGELOG.md)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20(Apple%20Silicon)-lightgrey.svg)](https://autonomic-ai.dev)
[![Documentation](https://img.shields.io/badge/docs-autonomic--ai.dev-black.svg)](https://autonomic-ai.dev)

Part of [Autonomic AI](https://autonomic-ai.dev), alongside [agent-spine](https://github.com/autonomic-ai-dev/agent-spine). Operates locally over stdio MCP. Route-gate hooks are installed for **Claude Code**, **Cursor**, **Antigravity (AGY)**, **OpenCode**, **Codex** and **Gemini CLI**. **VS Code** and GitHub Copilot get an MCP entry and instructions only.

---

## Architectural Overview

```mermaid
flowchart LR
    Host["AI Agent / IDE Host"] --> Gate{"route_gate<br/>(Host Pre-Tool Hook)"}
    Gate --> Engine[("SQLite + HNSW Engine<br/>Local Store")]
    Engine --> Context["Dynamic Knapsack (350–700 Tok)<br/>Skills · Rules · Invariants"]
    Engine --> Graph[("Procedure Graph<br/>Schema v26 Recursive CTEs")]
    Context --> Host
    Host --> Verify["agent-brain verify -- <cmd>"]
    Verify --> Trajectory[("Automatic Trajectory Capture<br/>& Outcome Attribution")]
```

---

## Why agent-brain?

| Challenge in Coding Agent Tooling | agent-brain Solution | Verified Mechanism |
| :--- | :--- | :--- |
| **Context Window Bloat** | Turn router packs the top-ranked skills, rules and memory into a fixed per-task token budget (350–700 tokens) | Knapsack token packer with a 120-token memory reservation (`engine.rs`) |
| **Instruction Drift & Ignored Prompts** | Host pre-tool hooks enforce `route_task` on every turn | Routing regression suite (`agent-brain/tests/jailbreak_routing.rs`, 250 cases) passes |
| **Destructive Shell Hazards** | The shell command policy blocks dangerous commands before they run | Command policy rules (`agent-brain/src/command_policy.rs`) |
| **Repetitive Execution Amnesia** | Procedure Graph (`recall_procedures`) stores structured, multi-step execution graphs | Schema v26 SQLite recursive CTEs (`procedure_edges`) |
| **Discarded Verification Evidence** | Automatic trajectory capture and outcome attribution (exit 0 vs non-zero) | `verify.rs` & `trajectory.rs` closed loop |
| **High Prompt Turn Latency** | Turn cache and session stickiness reuse recent routing decisions | `routing.turn_ttl_secs`, `routing.session_stickiness_secs` (`config.rs`) |

---

## Why teams adopt it

**1. Proof on day one.** `agent-brain dashboard` writes a local HTML page with total tokens saved by routing and bounded reads, route counts and latency. Run it after your first session:

```bash
agent-brain dashboard --days 7
```

`agent-brain stats` prints the route, cache and token-tool counters in the terminal. It does not print a tokens-saved total; the dashboard does.

**2. One install for every host.** One binary configures Claude Code, Cursor, Codex, Gemini CLI and OpenCode:

```bash
agent-brain install --all --global
```

Antigravity and VS Code are available per host (`--antigravity`, `--vscode`). Hook enforcement is not available on every host; see the Supported Runtimes table below.

**3. Local and private.** Routing, memory, verification and the token budget run on your machine against a local SQLite database. The route path makes no network calls. Outbound network happens only for the one-time embedding model download on first use, and for commands you run yourself: `learn`, `package add`, extension install and `sync cloud`. Nothing is sent to a hosted service by default.

---

## Supported Runtimes

| Runtime | Hook Mode | Integration Type |
| :--- | :--- | :--- |
| **Claude Code** | PreToolUse Hook | Native CLI stdio MCP + route gate |
| **Cursor** | `beforeShellExecution` hook | `.cursor/mcp.json` + `cursor-user-rules.mdc` |
| **Antigravity (AGY)** | BeforeTool Hook | Native CLI stdio MCP + route gate |
| **OpenCode** | Session / tool plugin | `route-gate.js` plugin (`agent-brain/hooks/opencode/`) |
| **Codex CLI / Gemini CLI** | Shell / tool hooks | Direct stdio MCP client |
| **VS Code / GitHub Copilot** | None (instructions only) | `.vscode/mcp.json` entry + Copilot instructions |

---

## Quickstart & Installation

### Option 1: Homebrew (macOS — Recommended)

```bash
brew tap autonomic-ai-dev/tap
brew install agent-brain
agent-brain install --global
```

### Option 2: Direct Download / Curl (Apple Silicon Developer ID–Signed)

```bash
curl -fsSL https://raw.githubusercontent.com/autonomic-ai-dev/agent-brain/master/scripts/install.sh | bash -s -- --global
```

### Option 3: Compile From Source (Rust)

```bash
git clone https://github.com/autonomic-ai-dev/agent-brain.git
cd agent-brain
cargo build --release -p agent-brain
./target/release/agent-brain install --global
```

### Verify Host Installation

```bash
agent-brain version
agent-brain doctor
```

`install --global` writes MCP configuration, hooks, and mode snippets. Paste `~/.agent_brain/cursor-user-rules.mdc` into Cursor **User Rules** if using Cursor. The host runtime spawns `serve` automatically over stdio.

To target specific agent runtimes:

```bash
agent-brain install --all --global
# or selectively: --claude-code | --cursor | --opencode | --gemini | --antigravity | --vscode | --codex
```

### Your first verify receipt

After install, run `agent-brain verify init` in a project, then `agent-brain verify -- <your test command>`. `agent-brain verify receipt` then reports whether that command passed for your current commit and code, and `--markdown` prints a block for a pull request. The full walk-through is in [docs/first-receipt.md](agent-brain/docs/first-receipt.md).

- Which hosts get which gates: [docs/support-matrix.md](agent-brain/docs/support-matrix.md)
- Every network path agent-brain can open, and its default: [docs/egress.md](agent-brain/docs/egress.md)

---

## Core Pillars

### 1. Dynamic Cognitive Governor (Token Knapsack)
Instead of dumping full documentation trees into your agent's system prompt, `agent-brain` intercepts each turn and calculates an exact token knapsack. Token budgets scale elastically by task kind:
- **Verification**: 350 tokens
- **Code Review**: 550 tokens
- **Implementation**: 600 tokens
- **Debugging**: 650 tokens
- **Architecture**: 700 tokens

A 120-token reservation is set aside for memory items in each route.

### 2. Procedure Graph & Associative Memory
Built on local SQLite with schema v26 migrations, `recall_procedures` queries multi-step engineering recipes. Connected downstream procedures and dependent execution nodes are dynamically hydrated using SQLite recursive Common Table Expressions (`procedure_edges`), ranked by task similarity and historical pass rate.

### 3. Deterministic Security & Host Hook Enforcement (`bash-gate`)
Agent execution is governed by these hooks:
- **`route_gate`**: Blocks agent tool calls until the turn has been routed with `route_task`.
- **`bash-gate`**: Inspects shell commands, including the contents of scripts they run (`bash`, `sh`, `source`). Blocks `rm -rf /` and force pushes before they run.

### 4. Closed-Loop Verification & Trajectory Attribution
Test runs executed through `agent-brain verify -- <cmd>` are recorded with their exit code and the hash of the code they tested:
- Verified recipes from passing runs are offered as `must_apply` items on later turns.
- Recipes whose latest run failed are withheld from recommendations.
- With `auto_trajectory` enabled, execution steps are captured in the `workflow_trajectory` table.

---

## CLI Reference

| Command | Description |
| :--- | :--- |
| `agent-brain install --global` | Configure MCP server, host hooks, and runtime permissions |
| `agent-brain doctor` / `--fix` | Diagnostic health check (validates binary, hooks, SQLite store) |
| `agent-brain briefing` | Summarizes last `route_task` decision and active token budget |
| `agent-brain stats` | Route, cache, token-tool and memory counters for the last period |
| `agent-brain dashboard [--days N]` | Writes the local HTML page with total tokens saved |
| `agent-brain index` | Indexes local skills, rules, and workspace memory into SQLite |
| `agent-brain verify -- <cmd>` | Execute and record verified command with trusted exit attribution |
| `agent-brain remember --text TEXT --tag TAG` | Store one durable fact or architectural decision |
| `agent-brain package add <pkg>` | Install skill packages from registry (e.g. `@supervisor`, `@matt-skills`) |
| `agent-brain sync git\|cloud` | Synchronize memory graph and skills across development machines |

---

## Configuration & Local Store

Configuration resides in `~/.autonomic/config.yaml` or `~/.autonomic/memory/config.json`:

```yaml
routing:
  prefer_agstack: true
  auto_trajectory: true
  turn_ttl_secs: 60
  session_stickiness_secs: 1800
```

- **Default Database**: `~/.autonomic/memory/brain.db` (override with `AGENT_BRAIN_HOME`).
- **Embedding Model**: Uses the local `all-MiniLM-L6-v2` model, loaded on first use and unloaded after 10 minutes of inactivity (`IDLE_UNLOAD_AFTER`, `embed.rs`).

---

## Verification & Development

Run the test suite and verify architectural boundaries:

```bash
# Verify boundaries and unit tests
agent-brain verify -- .verify/checks/core_boundary.sh
agent-brain verify -- cargo test -p agent-brain-core

# Full verification suite
agent-brain verify -- .verify/run.sh
```

---

## Ecosystem & Documentation

- **Documentation Portal**: [https://autonomic-ai.dev](https://autonomic-ai.dev)
- **Memory & Routing Architecture**: [https://autonomic-ai.dev/docs/memory](https://autonomic-ai.dev/docs/memory)
- **Ecosystem Architecture Guide**: [https://autonomic-ai.dev/docs/architecture](https://autonomic-ai.dev/docs/architecture)
- **Workflows & Procedures**: [https://autonomic-ai.dev/docs/workflows](https://autonomic-ai.dev/docs/workflows)

---

## License

MIT © [Autonomic AI](https://github.com/autonomic-ai-dev) · See [CHANGELOG.md](CHANGELOG.md) for release history.
