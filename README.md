# agent-brain

Fast, local MCP server that routes each turn to the right **agents, skills, rules, and memory** under a strict token budget.

Rust is the brain; Cursor/Claude are the hands.

## Quick install (another laptop)

**Option A — one-liner** (downloads latest release binary, configures Cursor):

```bash
curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/main/scripts/install.sh | bash -s -- --global
```

**Option B — build from source** (requires Rust):

```bash
cargo install --git https://github.com/aeswibon/agent-brain --locked agent-brain
agent-brain install --global
agent-brain index
```

**Option C — manual MCP config** after building locally:

```bash
cargo build --release -p agent-brain
./target/release/agent-brain install --global
```

Restart Cursor and enable the `agent-brain` server under **Settings → MCP**.

## MCP config shape

```json
{
  "mcpServers": {
    "agent-brain": {
      "command": "/Users/you/.local/bin/agent-brain",
      "args": ["serve"],
      "env": {
        "RUST_LOG": "agent_brain=info"
      }
    }
  }
}
```

`agent-brain install` writes this using the absolute path of the current binary.

## Commands

| Command | Description |
|---------|-------------|
| `agent-brain serve` | Start MCP server (stdio) |
| `agent-brain index` | Reindex local agents/skills/rules/memory |
| `agent-brain install` | Write `.cursor/mcp.json` in current directory |
| `agent-brain install --global` | Write `~/.cursor/mcp.json` |

## Data directory

Default: `~/.agent_brain/` — override with `AGENT_BRAIN_HOME`.

First run downloads the `AllMiniLML6V2` embedding model (~90MB).

## Development

```bash
cargo test --release -p agent-brain
cargo build --release -p agent-brain
```

See [agent-brain/README.md](agent-brain/README.md) for tool details.

## CI & releases

- Every push: tests + build artifacts (30-day retention)
- Tags `v*`: GitHub Release with macOS/Linux/Windows binaries

Download a CI artifact or release asset, then:

```bash
chmod +x agent-brain-*
mv agent-brain-* ~/.local/bin/agent-brain
agent-brain install --global
```

## License

MIT
