# agent-brain verification playbooks

Project-root `.verify/` is discovered by `agent-brain verify` / `report` for scenario hashing.

- **`verify.toml`** — command allowlist. Only matching commands count as trusted for future learning. `agent-brain verify -- true` does not match and earns no learning credit.
- Record outcomes with `agent-brain verify -- <cmd>` (source=`cli`) or spine/muscle with `--source orchestrator`.
- Prefer `verify --` over MCP `record_verify_outcome` (agent-asserted exits are not trusted for learning).
