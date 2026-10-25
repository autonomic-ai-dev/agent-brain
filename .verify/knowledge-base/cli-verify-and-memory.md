# cli-verify-and-memory knowledge base

## Owned code

- `agent-brain/src/mcp/server.rs`: the MCP tool surface, including `GrepSearchParams`.
- `agent-brain/src/git_tools.rs`: the `brain_git_*` tools. No other feature map owns them, so this feature owns them by assignment.

## Behaviour that matters

- `grep_search` accepts `query` as an alias for `pattern`. `path` defaults to `"."`. Callers that send `query` no longer get `missing field 'pattern'`.
- The git tools run `git` with the working directory set to the given path. Git does not climb past a mount boundary, so a non-repository path under `/Volumes/...` fails. The error now tells the caller to pass the repository root in `current_working_directory`.
- `GIT_DISCOVERY_ACROSS_FILESYSTEM` is deliberately not set globally. Setting it would let git find an unrelated parent repository.

## Verification

- `agent-brain verify -- cargo test --all-features` in `agent-brain/agent-brain`: exit 0, recorded. Regression tests: `git_log_outside_repo_hints_to_pass_repository_root` and `grep_search_params_accept_query_alias_and_default_path`.
- Feature map `cli-verify-and-memory.md` lists `agent-brain/src/mcp/server.rs` and `agent-brain/src/git_tools.rs` under `## Code`.
- Open: `route_task` does not return tool parameter shapes. That needs an index format change and API approval, and is not implemented.
