# 7. Enforcement and multi-host integration

## Summary

**Cursor**, **Claude Code**, **OpenCode**, **Codex**, and **Gemini/Antigravity** install route gates (hooks or plugins) that deny other agent-brain MCP tools until `route_task` succeeds. **VS Code** and **Claude Desktop** still rely on instruction files only.

## What we built

### Cursor hooks (`hooks/route_gate.py`)

Installed by `agent-brain install --global` into `~/.cursor/hooks.json`:

| Hook | Behavior |
|------|----------|
| `beforeMCPExecution` | Track `route_task` success / failure |
| `preToolUse` | Deny tools until routed for current user turn |

State persisted in `~/.autonomic/memory/hooks/route_state.json` (legacy: `~/.agent_brain/hooks/`).

### Gate scope (`AGENT_BRAIN_ROUTE_GATE_SCOPE`)

| Value | Behavior |
|-------|----------|
| `brain_mcp` (default) | Only gate agent-brain MCP tools until `route_task`; Shell/Read keep working if MCP offline |
| `all` | Legacy strict mode — gate every tool |

**Why scoped gate:** v0.7.1+ — MCP disconnect should not freeze the entire session. Offline cooldown (`AGENT_BRAIN_ROUTE_OFFLINE_SECS`) eventually allows proceed-with-warning.

### Grace and stale routing

- **Grace period** after failed `route_task` — avoid infinite lock if MCP errors
- **Stale route timeout** — re-route if last success too old within same turn

### Permissions (`~/.cursor/permissions.json`)

`install --global` adds `agent-brain:*` to MCP allowlist so Cursor CLI agents skip per-session approval prompts.

### Multi-host install (`host_install.rs`)

| Host | Command | Config + gate |
|------|---------|---------------|
| Cursor | `install --global` | `~/.cursor/mcp.json` + hooks |
| Claude Code | `install --claude-code [--global]` | `.mcp.json` / `~/.claude.json` + settings hooks + hard rule |
| OpenCode | `install --opencode [--global]` | `opencode.json` + plugin + rules |
| Codex | `install --codex [--global]` | `~/.codex/config.toml` + hooks |
| Gemini | `install --gemini [--global]` | `~/.gemini/settings.json` + hooks |
| Antigravity CLI | `install --antigravity [--global]` | antigravity MCP configs + shared Gemini hooks + hard rule |
| VS Code | `install --vscode [--global]` | mcp.json + Copilot instructions (advisory) |
| Claude Desktop | `install --claude-desktop` | `claude_desktop_config.json` (advisory) |
| All | `install --all --global` | Above combined |

Each gated host also gets instruction / hard-rule files (e.g. `agent-brain.md`, `rules/agent-brain-route.md`). OpenCode JSON uses `mcp.agent-brain.type: local` and `command: [binary, serve]`.

### Doctor

`agent-brain doctor` reports Cursor MCP/hooks/codesign plus OpenCode, Claude Code, Antigravity, Codex, and Gemini gate status. `--fix` reinstalls stale matchers, missing plugins, and shared hooks.

## Host enforcement matrix

| Host | Mechanism | Default scope |
|------|-----------|---------------|
| Cursor | `hooks.json` → `route_gate.py` | `brain_mcp` |
| Claude Code | `settings.json` PreToolUse/PostToolUse (`matcher: .*`) + hard rule | `brain_mcp` inside script |
| OpenCode | `agent-brain-route-gate.ts` plugin + `rules/agent-brain-route.md` | brain tools fail-closed |
| Gemini / Antigravity CLI | shared `~/.gemini/settings.json` BeforeAgent/BeforeTool | `.*` → script scope |
| Codex | `~/.codex/hooks.json` | PreToolUse |
| VS Code / Claude Desktop | instruction files only | advisory |

**Implication:** Enforcement parity now covers the major coding CLIs; VS Code / Claude Desktop remain advisory until those hosts expose deny hooks.

## Alternatives considered

### Rely on rules only (no hooks)

**Rejected:** Insufficient — models skip optional steps under pressure.

### Gate all tools always (`scope=all`)

**Shipped as opt-in:** Too brittle when MCP restarts or user needs emergency shell.

### Custom Cursor extension instead of hooks

**Rejected:** Hooks are the supported 2025+ mechanism; extension maintenance burden.

### OpenCode fork / patch

**Rejected:** Upstream plugin + MCP install is maintainable; a fork is not.

### Workspace-only MCP (no global)

**Default Cursor install is global:** Skills are user-level; project-only MCP would miss `~/.cursor/skills` index context unless cwd bootstrap covers it.

## Trade-offs

- **Advisory hosts (VS Code / Claude Desktop):** Discipline depends on instructions — verify with `last-route.md` / MCP logs.
- **Tool name drift:** Gates must recognize Cursor (`mcp_agent-brain_*`), Claude (`mcp__agent-brain__*`), and OpenCode (`agent-brain_*`) forms.
- **Python hook dependency:** Hosts invoke `route_gate.py` with system Python; kept stdlib-only. OpenCode wraps it in a TS plugin.

## For senior engineers and principal architects

### Enforcement is a product feature, not an implementation detail

On Cursor, Claude Code, OpenCode, Codex, and Gemini/Antigravity, **routing is mandatory** before other agent-brain tools (within gate scope). VS Code / Claude Desktop remain **advisory**.

PE implication: run `agent-brain doctor` (and `--fix`) so OpenCode plugin files, Claude matcher upgrades, and Antigravity shared hooks stay current. Spot-check `~/.agent_brain/logs/last-route.md`.

### Scoped gate (`brain_mcp`) rationale

`scope=all` blocked Shell/Read when MCP flapped — developers could not recover without disabling hooks. **`brain_mcp` scope** says: “You must route before calling agent-brain tools; other tools remain available if MCP is down.”

This is a **degraded-mode** design familiar from distributed systems: prefer partial availability over total lockout.

### Offline grace and stale route

| Mechanism | Purpose |
|-----------|---------|
| `AGENT_BRAIN_ROUTE_OFFLINE_SECS` | After MCP failures, allow proceed with warning |
| Stale route timeout | Re-route within long turns if context shifted |

Without grace, a single MCP crash **bricks the agent session** — unacceptable for daily driver IDE use.

### Multi-host install matrix

`install --all --global` exists because PEs asked for **one command** to align Cursor + OpenCode + Claude. Each host has different JSON shapes (`opencode.json` vs `mcp.json`); `host_install.rs` centralizes that knowledge.

**macOS codesign** is part of “install works” — unsigned binaries die under taskgated when Cursor spawns MCP (see [10](10-concurrency-and-performance.md)).

### Failure modes

| Symptom | Cause | Action |
|---------|-------|--------|
| Infinite deny loop | `route_task` errors every turn | Fix MCP binary; check `doctor` |
| OpenCode denies all brain tools | Plugin missing `route_gate.py` | `install --opencode --global` |
| Claude `stale (reinstall)` | Old `mcp__agent-brain__.*` matcher | `doctor --fix` |
| Antigravity MCP ok, no gate | Shared Gemini hooks missing | `install --antigravity --global` |
| Wrong tool name in gate | Host renamed MCP tools | Update `route_gate.py` / OpenCode plugin patterns |
| Hooks missing after update | Install not re-run | `install --all --global` or `doctor --fix` |

### Questions a PE should ask

1. What **% of traffic** is on gated CLIs vs advisory hosts (VS Code / Claude Desktop)?
2. Is **degraded proceed without route** acceptable under MCP outage?
3. Do security policies allow **global MCP** configs?
4. Who re-runs **`doctor --fix`** when host hook schemas change?

## Further reading

- [host-integration.md](../host-integration.md)
- [route_gate.py](../../agent-brain/hooks/route_gate.py)
- [install.rs](../../agent-brain/src/install.rs)
