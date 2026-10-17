# agent-brain — Context for agents

Local turn router and verification ledger for the Autonomic stack. Part of [agent-body](https://github.com/autonomic-ai-dev/agent-body).

| Field | Value |
|-------|-------|
| Role | Context / memory / route gate (PV analog) |
| Version | v1.0.0-beta.3.1 |
| Docs Hub | [https://autonomic-ai.dev/docs/memory](https://autonomic-ai.dev/docs/memory) · [Architecture](https://autonomic-ai.dev/docs/architecture) |
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

## Prefer agent-body

For efficiency, organ work goes through **agent-body** (`autonomic serve-mcp` / `route_to_mcp` server `agent-body`): execute (`muscle_*`), workflows (`spine_*`), budget (`heart_*`), scan/sandbox (`immune_*`), UI (`eyes_*`), approvals (`mouth_*`). Catalog: `agent-brain registry list`.

## Default skills

`@matt-skills` ([mattpocock/skills](https://github.com/mattpocock/skills)) installs on `doctor --fix` / install, is indexed under `packages/skills/`, and receives `recommend_boost: 0.35` in routing.

## Key tools / CLI

- MCP: `route_task`, `grep_search`, `file_summary`, `read_file_head`/`tail`, `store_memory`
- CLI: `doctor`, `verify`, `report`, `memory put|enqueue|flush-pending`, `install --global`

## CI

Push path filter + PR skip for docs-only diffs. Protected paths require label `protected-path-approved`. Pattern: [ci-path-filter-pattern.md](ci-path-filter-pattern.md).

## Related

Docs Hub: [https://autonomic-ai.dev/docs/memory](https://autonomic-ai.dev/docs/memory) · Architecture: [https://autonomic-ai.dev/docs/architecture](https://autonomic-ai.dev/docs/architecture)
