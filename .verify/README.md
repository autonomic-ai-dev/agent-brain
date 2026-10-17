# agent-brain verification playbooks

Project-root `.verify/` is discovered by `agent-brain verify` / `report` for scenario hashing.

- **`verify.toml`** — command allowlist. Only matching commands count as trusted for future learning. `agent-brain verify -- true` does not match and earns no learning credit.
- Record outcomes with `agent-brain verify -- <cmd>` (source=`cli`) or spine/muscle with `--source orchestrator`.
- Prefer `verify --` over MCP `record_verify_outcome` (agent-asserted exits are not trusted for learning).

## Playbooks

| Playbook | Command | Purpose |
|----------|---------|---------|
| **Full Suite** | `agent-brain verify -- .verify/playbooks/verify_full_suite.sh` | Runs all 195+ unit & integration tests with all features enabled |
| **Routing & Adherence** | `agent-brain verify -- .verify/playbooks/verify_routing.sh` | Validates route_task, verified recipe elevation, and automated context feedback |
| **Token Tools** | `agent-brain verify -- .verify/playbooks/verify_token_tools.sh` | Tests bounded file readers, grep searches, and token saving benchmarks |
| **Default Suite** | `agent-brain verify -- .verify/run.sh` | Complete test & cargo check verification entrypoint |

