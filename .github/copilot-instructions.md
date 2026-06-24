# agent-brain MCP (GitHub Copilot / VS Code)
instructions-version: 5

VS Code and GitHub Copilot do not expose Cursor-style PreToolUse hooks. Enforcement is:

1. Connect agent-brain MCP (`agent-brain install --vscode [--global]`).
2. Call **`route_task`** at the start of every user turn before planning or edits.
3. Use agent-brain token tools (`grep_search`, `file_summary`, `read_file_head/tail`) instead of unbounded workspace search.
4. Call **`store_memory`** at task end for durable outcomes (max 50 words, no secrets).

Session digests from Cursor, OpenCode, Codex, Gemini, and Antigravity only surface through `route_task`.
