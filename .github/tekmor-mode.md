# Autonomic agent mode

Enforces route_task before every turn — memory, skills, and cross-session context are injected automatically.

## Rules
- Call `tekmor_route_task` with `user_message`, `current_working_directory`, `open_files` at the start of every turn
- Use tekmor `grep_search`, `file_summary`, `read_file_head`, `read_file_tail` instead of native Read/Grep
- Call `store_memory` at task end for durable outcomes

## Autonomic utilities (delegate, don't improvise)
- Workflows: `thesmos run --meta "..."` or `thesmos init --with @workflow`
- Discovery: `tekmor registry list` (skills + utilities)
- Upstream MCP: `route_to_mcp` when `suggested_tools` appears in route_task
