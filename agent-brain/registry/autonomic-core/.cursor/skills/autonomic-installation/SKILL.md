---
name: autonomic-installation
description: Guide for installing, updating, and managing Autonomic AI organs and resolving binary installation and PATH conflicts. Use when installing organs, troubleshooting missing commands, or fixing PATH shadowing between Homebrew and ~/.local/bin.
---

# Autonomic AI Installation & PATH Conflict Resolution

## 1. Recommended Installation (Homebrew on macOS)

On macOS (Apple Silicon arm64), the official and primary installation method is Homebrew:

```bash
# Tap the repository
brew tap autonomic-ai-dev/tap

# Install the full organ stack (all 10 organs + NATS server)
brew install autonomic-stack

# Or install individual organs as needed:
brew install autonomic       # Meta CLI & organ supervisor
brew install agent-brain     # Cognitive graph, MCP server, long-term memory
brew install agent-spine     # Workflow DAG execution controller
brew install agent-heart     # Budget gate & heartbeat monitor
brew install agent-nerves    # NATS / JetStream pub-sub event bridge
brew install agent-muscle    # Sandboxed execution & training daemon
brew install agent-immune    # Security vulnerability scanner & sandbox
brew install agent-eyes      # Screen capture & DOM indexer
brew install agent-mouth     # AST validator & Slack approvals
brew install agent-tui       # Terminal user interface dashboard
```

## 2. Resolving Binary Installation & PATH Conflicts

### The Problem: PATH Shadowing

If you previously installed organs via `curl -fsSL ... | bash` or built them locally with `cargo install`, older binaries are placed in `~/.local/bin` or `~/.cargo/bin`.

If `~/.local/bin` appears before `/opt/homebrew/bin` in your shell's `PATH`:
- Shell commands run the **outdated** binary in `~/.local/bin/` instead of the newly installed Homebrew version.
- Common symptoms:
  - New features or CLI flags fail with "unknown command" or "unknown package alias" (e.g. `agent-brain add @matt-skills` fails on an old binary).
  - Version mismatch between `brew list` and `agent-brain --version`.

### The Fix

1. **Remove shadowed binaries from `~/.local/bin`**:
   ```bash
   rm -f ~/.local/bin/{autonomic,agent-body,agent-brain,agent-spine,agent-heart,agent-nerves,agent-muscle,agent-immune,agent-eyes,agent-mouth,agent-tui}
   ```

2. **Verify active binaries point to Homebrew**:
   ```bash
   which autonomic agent-brain agent-spine
   # Expected output: /opt/homebrew/bin/...
   ```

3. **Ensure symlinks are cleanly linked in Homebrew**:
   ```bash
   brew link --overwrite autonomic agent-brain agent-spine agent-heart agent-nerves agent-muscle agent-immune agent-eyes agent-mouth agent-tui
   ```

4. **Verify version matches latest release**:
   ```bash
   agent-brain --version
   autonomic --version
   ```

## 3. Post-Installation Workspace Initialization

After installation or upgrade:

```bash
# 1. Initialize workspace
autonomic init

# 2. Register global MCP hooks for Cursor, Claude Desktop, VS Code, etc.
agent-brain install --global

# 3. Verify health across all organs
autonomic doctor

# 4. Start daemon stack (NATS broker + nerves + heart)
autonomic start
```

## 4. Alternative: Standalone Linux or Manual Script Install

For Linux or environments without Homebrew:

```bash
# Complete stack
curl -fsSL https://raw.githubusercontent.com/autonomic-ai-dev/agent-body/master/scripts/install-all-organs.sh | bash

# Meta CLI only
curl -fsSL https://raw.githubusercontent.com/autonomic-ai-dev/agent-body/master/scripts/install.sh | bash
```
