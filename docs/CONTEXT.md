# agent-brain — Context for agents

Local turn router and verification ledger for the Autonomic stack. Part of [agent-body](https://github.com/autonomic-ai-dev/agent-body).

| Field | Value |
|-------|-------|
| Role | Context / memory / route gate (PV analog) |
| Binary | `agent-brain` (MCP stdio) |
| Version | 0.40.0+ |
| Home | `~/.autonomic/memory/` (override `AGENT_BRAIN_HOME`) |

## Mental model

Each user turn: call **`route_task`** once → load ranked skills/rules/memory → work → **`store_memory`** at task end. Hooks block other brain MCP tools until `route_task` succeeds.

## Verification (trusted path)

| Source | How | Learning trust |
|--------|-----|----------------|
| `orchestrator` | `agent-brain verify --source orchestrator -- <cmd>` | Trusted |
| `cli` | `agent-brain verify -- <cmd>` | Trusted if allowlisted |
| `hook` | Shell PostToolUse auto-record | Observational / allowlisted |
| `mcp` / `playbook` | `record_verify_outcome` | **Not** trusted for learning |

Allowlist: project-root `.verify/verify.toml` (`[verify].commands`). Prefer **`verify --`** over MCP asserts.

## Key tools / CLI

- MCP: `route_task`, `grep_search`, `file_summary`, `read_file_head`/`tail`, `store_memory`
- CLI: `doctor`, `verify`, `report`, `memory put|enqueue|flush-pending`, `install --global`

## CI

Push path filter + PR skip for docs-only diffs. Protected paths require label `protected-path-approved`. Pattern: [ci-path-filter-pattern.md](ci-path-filter-pattern.md).

## Related

Operator usage: [USAGE.md](USAGE.md) · Slim contract: [AGENT-CONTEXT.md](AGENT-CONTEXT.md) · Ecosystem deep dive: agent-body `docs/CONTEXT.md`
