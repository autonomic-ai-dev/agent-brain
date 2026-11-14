# Host support matrix

`tekmor install` configures each supported coding host. This page states, for each host, what the installer writes and which automated tests check it. Read it as: "the install writes these files; the tests named here check them".

Install with `tekmor install --all` (the default), or pick one host with a flag. Add `--global` to write user-level config instead of project-level config.

| Host | Flag | MCP server entry | Route and gate hooks | Instruction files | Automated tests |
|---|---|---|---|---|---|
| Claude Code | `--claude-code` | `.mcp.json` at the project root, or `~/.claude.json` with `--global` | `UserPromptSubmit`, `PreToolUse` and `PostToolUse` in `.claude/settings.json` | `.claude/rules/tekmor-route.md` and `.claude/rules/tekmor-agstack.md` (agstack policy, including the verification step) | `merges_claude_json_user_scope` (unit); `claude_code_project_uses_mcp_json_at_root` (`tests/v0_9.rs`) |
| Cursor | `--cursor` | `.cursor/mcp.json` in the workspace, or `~/.cursor/mcp.json` with `--global` | Shell gate on `beforeShellExecution` in the Cursor hooks file | `.mdc` rule files under `~/.tekmor/` | Hook assertion in `src/install.rs`; `tests/v0_9.rs`, `tests/v0_10.rs`, `tests/jailbreak_routing.rs` |
| OpenCode | `--opencode` | `opencode.json` at the project root, or the user config with `--global` | Route-gate plugin `tekmor-route-gate.js` | OpenCode instructions file | `merges_opencode_config_preserves_schema` (unit); `opencode_project_uses_opencode_json_at_root` (`tests/v0_9.rs`) |
| Codex | `--codex` | A TOML block in the Codex config (`~/.codex/config.toml` with `--global`) | `hooks.json` in `~/.codex/` or `.codex/` | Codex instructions file | `merges_codex_config_appends_when_missing` and `merges_codex_config_replaces_existing_section` (unit) |
| Gemini CLI | `--gemini` | `~/.gemini/settings.json` or `.gemini/settings.json` | `BeforeAgent` and `BeforeTool` in `settings.json` | Gemini instructions file | `gemini_project_uses_settings_json` and `merges_gemini_settings_preserves_other_keys` (unit) |
| Antigravity | `--antigravity` | Antigravity config paths (`antigravity_config_paths`) | Shares `~/.gemini/settings.json` hooks | Antigravity instructions file | `antigravity_global_paths_include_shared_config` (unit) |
| VS Code (Copilot) | `--vscode` | `mcp.json` under the VS Code settings folder | None. No gate runs before VS Code actions. | Copilot instructions file | `merges_vscode_servers` (unit); `vscode_workspace_path_under_vscode_dir` (`tests/v0_9.rs`) |
| Claude Desktop | `--claude-desktop` | `claude_desktop_config.json` | None | None | `claude_desktop_path_is_absolute_file` (`tests/v0_9.rs`) |

## Manual step for Cursor

`tekmor install` writes `~/.tekmor/cursor-tekmor-mode.mdc` and `~/.tekmor/cursor-user-rules.mdc`. The installer prints a request to paste both files into Cursor's User Rules, and it does not do that paste itself, so this step is manual.

## Mode in project installs

A project install (no `--global`) writes the agstack mode into the file each host reads for project instructions. The text sits between two markers, `<!-- tekmor:begin mode -->` and `<!-- tekmor:end mode -->`:

| Host | File |
|---|---|
| Codex, OpenCode, Antigravity | `AGENTS.md` at the repository root |
| Gemini CLI | `GEMINI.md` at the repository root |
| VS Code (Copilot) | `.github/copilot-instructions.md` |
| Claude Code | `.claude/rules/tekmor-agstack.md` |
| Cursor | `.cursor/rules/agstack.mdc` (always applied) and `.cursor/rules/tekmor-mode.mdc` |

Text outside the markers is never changed, and a second install leaves the file as it was. The file names follow each host's usual convention. The regression test is `tests/project_mode.rs`.

## What "hooks" means here

Hooks run tekmor's gates before a host action. The bash gate blocks unsafe shell commands. The route gate requires `route_task` before work starts. Only the hosts marked with hooks above get these gates. VS Code and Claude Desktop get the MCP tools and the instructions, but no gate runs before their actions.

## Known gaps

- Codex has install and config-merge unit tests, but no end-to-end test that runs the Codex hooks.
- VS Code and Claude Desktop have no gates, so nothing stops an unsafe action before it runs there. Receipts still work, because they come from commands you run with `tekmor verify --`.
- Claude Code and Codex ask you to trust new hooks. The install output says so: Claude Code in `/hooks`, Codex after a restart or with `/hooks`.

## Changing this page

When `src/host_install.rs` or `src/install.rs` changes what a host gets, update its row here in the same commit. Each row names a test. If you remove a test, remove its row or name the test that replaces it.
