---
name: rmcp-mcp-gateway
description: Rust rmcp MCP servers for Autonomic AI — agent-body serve-mcp gateway, agent-brain route_to_mcp federation, tool_router/tool_handler macros, JSON Schema inputSchema, Cursor and OpenCode mcp.json registration, stdio transport debugging.
---

# Rust MCP Gateway (Autonomic / rmcp)

Use when building or debugging **Rust MCP servers** in the Autonomic stack: `agent-body serve-mcp`, `agent-brain serve`, organ `serve-mcp` binaries, or `route_to_mcp` upstream federation.

## When to Use

- MCP tools missing in **Cursor** or **OpenCode** (empty `inputSchema`, wrong `serverInfo`)
- Implementing or fixing `autonomic serve-mcp` / organ MCP gateways
- Comparing **agent-brain** vs **agent-body** vs **agent-muscle** MCP patterns
- Debugging `tools/list`, `tools/call`, `initialize`, or `route_to_mcp`
- Adding tools to the gateway or an organ MCP server

## Working Pattern (rmcp 1.7+)

Match **agent-brain** and **agent-muscle**:

1. Define param structs with `schemars::JsonSchema` + `serde::Deserialize`
2. `#[tool_router] impl Server { #[tool(...)] async fn tool_name(&self, params: Parameters<T>) ... }`
3. `#[tool_handler] impl ServerHandler for Server { fn get_info() -> ServerInfo with Implementation name/version }`
4. `server.serve(rmcp::transport::io::stdio()).await?`

**Do not** hand-roll `Tool::new(name, desc, empty_schema)` — hosts require JSON Schema draft 2020-12 with `properties` and `required`.

## Verify Registration

```bash
# List tools + schemas over stdio
python3 - <<'PY'
import json, subprocess
p = subprocess.Popen(["agent-body","serve-mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
for m in [
  {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}},
  {"jsonrpc":"2.0","method":"notifications/initialized"},
  {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}},
]:
    p.stdin.write(json.dumps(m)+"\n")
p.stdin.close()
for line in p.stdout:
    r = json.loads(line)
    if r.get("id")==1: print("serverInfo:", r["result"]["serverInfo"])
    if r.get("id")==2:
        t = next(x for x in r["result"]["tools"] if x["name"]=="muscle_execute_bash")
        print("schema:", t["inputSchema"])
PY
```

Expect `$schema`, `properties`, `required`, and `serverInfo.name == "agent-body"`.

## Architecture

| Binary | Role |
|--------|------|
| **agent-brain** | Routing, memory, skills; `route_task` + `route_to_mcp` federation |
| **agent-body** | Gateway aggregating organ tools (`heart_*`, `muscle_*`, `spine_*`, …) |
| **agent-muscle / agent-spine / …** | Organ MCP servers spawned by gateway |

Config: `~/.autonomic/config.toml` → `[brain.upstream_mcp]` for `route_to_mcp` child servers.

Host config: `~/.cursor/mcp.json` and OpenCode `mcp` block — both can register `agent-brain` and `agent-body` separately.

## Common Failures

| Symptom | Likely cause |
|---------|----------------|
| Tools work via `route_to_mcp` but not direct MCP | Empty `inputSchema` on gateway; rebuild with `tool_router` |
| `serverInfo.name` is `rmcp` | Missing `Implementation` in `get_info`; stale binary |
| Hook blocks `route_to_mcp` | Call `route_task` first each turn |
| OpenCode shows no tools | Wrong binary path; restart MCP after install |

## Related Skills

- **mcp-server-patterns** (ECC) — general MCP concepts, Node/TS SDK, transport choice
- **execution-supervisor** — `route_task` every turn, token-efficient reads
