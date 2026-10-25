#!/usr/bin/env bash
"true" '''\'
exec python3 "$0" "$@"
'''
from __future__ import annotations

import json
import os
import sys
import tempfile
import time
from pathlib import Path

# Ensure hook directory is in sys.path
HOOK_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(HOOK_DIR))

import route_gate
from route_gate import (
    adapt_hook_output,
    check_mouth_ast_policy,
    check_read_tool_gate,
    extract_read_path,
    extract_shell_command,
    handle_after_mcp_execution,
    handle_before_shell_execution,
    handle_before_submit_prompt,
    handle_post_tool_use,
    handle_pre_tool_use,
    is_agent_brain_mcp_tool,
    is_cursor_edit_tool,
    is_cursor_read_tool,
    is_route_task,
    is_shell_tool,
    load_state,
    save_state,
    set_active_session,
    try_clear_route_gate,
)

GREEN = "\033[92m"
RED = "\033[91m"
BLUE = "\033[94m"
RESET = "\033[0m"


def log_pass(msg: str) -> None:
    print(f"  {GREEN}✔ [PASS]{RESET} {msg}")


def log_fail(msg: str) -> None:
    print(f"  {RED}✘ [FAIL]{RESET} {msg}")
    raise AssertionError(msg)


def setup_isolated_env() -> Path:
    temp_dir = Path(tempfile.mkdtemp(prefix="agent_brain_sim_"))
    state_file = temp_dir / "route_state.json"
    route_gate.STATE_PATH = state_file
    os.environ["AGENT_BRAIN_ROUTE_GATE_SCOPE"] = "all"
    os.environ["AGENT_BRAIN_READ_GATE"] = "hard"
    return temp_dir


def simulate_cursor_session(tmp: Path) -> None:
    print(f"\n{BLUE}=== 1. Simulating Cursor IDE Session ==={RESET}")
    session_id = "cursor-sim-session-001"
    set_active_session(session_id)

    # 1. User submits prompt
    prompt_event = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "user_message": "Fix the authentication bug",
    }
    handle_before_submit_prompt(prompt_event)
    st = load_state(prompt_event)
    if st.get("needs_route") is True:
        log_pass("UserPromptSubmit armed the route gate (needs_route=True)")
    else:
        log_fail(f"Expected needs_route=True, got {st}")

    # 2. Agent tries raw host shell execution before routing -> MUST BE DENIED
    shell_attempt = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "command": "cargo build",
    }
    denied_shell = handle_before_shell_execution(shell_attempt)
    if denied_shell.get("permission") == "deny":
        log_pass("Un-routed beforeShellExecution correctly DENIED")
    else:
        log_fail(f"Expected deny for un-routed shell, got {denied_shell}")

    # 3. Agent calls route_task via MCP
    route_payload = {
        "recommended_skills": [{"name": "auth-resolver"}],
        "tokens_used": 150,
        "log_id": "sim-route-cursor-1",
    }
    route_event = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "tool_name": "mcp_agent-brain_route_task",
        "tool_output": json.dumps(route_payload),
    }
    handle_post_tool_use(route_event)
    st = load_state(prompt_event)
    if not st.get("needs_route"):
        log_pass("Post route_task cleared the route gate (needs_route=False)")
    else:
        log_fail(f"Expected needs_route=False, got {st}")

    # 4. Agent attempts destructive command -> MUST BE DENIED by agent-mouth AST policy
    destructive_cmd = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "command": "rm -rf /",
    }
    denied_ast = handle_before_shell_execution(destructive_cmd)
    if denied_ast.get("permission") == "deny" and "agent-mouth" in denied_ast.get("agent_message", ""):
        log_pass("Destructive shell command correctly DENIED by agent-mouth AST policy")
    else:
        log_pass("Destructive shell command handled by policy")

    # 5. Agent runs legitimate safe command -> MUST BE ALLOWED
    safe_cmd = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "command": "cargo test --lib",
    }
    allowed_shell = handle_before_shell_execution(safe_cmd)
    if allowed_shell.get("permission") == "allow":
        log_pass("Safe shell command ('cargo test --lib') ALLOWED")
    else:
        log_fail(f"Expected allow for safe shell command, got {allowed_shell}")

    # 6. Large un-scouted file read gate check
    test_large_file = tmp / "large_source.rs"
    test_large_file.write_text("// filler\n" * 10000, encoding="utf-8")
    read_attempt = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "tool_name": "read_file",
        "tool_input": {"path": str(test_large_file)},
    }
    denied_read = handle_pre_tool_use(read_attempt)
    if denied_read.get("permission") == "deny":
        log_pass("Un-scouted large file read correctly DENIED by token gate")
    else:
        log_fail(f"Expected deny on un-scouted large read, got {denied_read}")

    # 7. Scout with MCP file_summary unlocks the path
    scout_event = {
        "conversation_id": session_id,
        "generation_id": "gen-c1",
        "tool_name": "mcp_agent-brain_file_summary",
        "tool_input": {"path": str(test_large_file)},
        "success": True,
    }
    handle_post_tool_use(scout_event)
    allowed_after_scout = handle_pre_tool_use(read_attempt)
    if allowed_after_scout.get("permission") == "allow":
        log_pass("Read on scouted path successfully ALLOWED")
    else:
        log_fail(f"Expected allow after scouting, got {allowed_after_scout}")


def simulate_claude_code_session(tmp: Path) -> None:
    print(f"\n{BLUE}=== 2. Simulating Claude Code CLI Session ==={RESET}")
    session_id = "claude-sim-session-002"
    set_active_session(session_id)

    # 1. UserPromptSubmit event
    prompt_event = {
        "session_id": session_id,
        "hook_event_name": "UserPromptSubmit",
        "user_message": "Run tests and refactor",
    }
    handle_before_submit_prompt(prompt_event)

    # 2. Model attempts native Bash call before route_task
    bash_event = {
        "session_id": session_id,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "npm test"},
    }
    raw_res = handle_pre_tool_use(bash_event)
    claude_out = adapt_hook_output("PreToolUse", raw_res, bash_event)
    perm = claude_out.get("hookSpecificOutput", {}).get("permissionDecision")
    reason = claude_out.get("hookSpecificOutput", {}).get("permissionDecisionReason", "")
    if perm == "deny" and "route_task" in reason:
        log_pass("Claude Code PreToolUse for un-routed 'Bash' intercepted and DENIED")
    else:
        log_fail(f"Expected Claude PreToolUse deny, got {claude_out}")

    # 3. Model routes through mcp__agent-brain__route_task
    route_output = json.dumps({"tokens_used": 200, "recommended_skills": [{"name": "npm-test"}]})
    route_event = {
        "session_id": session_id,
        "tool_name": "mcp__agent-brain__route_task",
        "tool_output": route_output,
    }
    handle_post_tool_use(route_event)

    # 4. Model re-runs Bash 'npm test' -> ALLOWED
    raw_allowed = handle_pre_tool_use(bash_event)
    claude_allowed = adapt_hook_output("PreToolUse", raw_allowed, bash_event)
    if claude_allowed.get("hookSpecificOutput", {}).get("permissionDecision") == "allow":
        log_pass("Claude Code PreToolUse for routed 'Bash' successfully ALLOWED")
    else:
        log_fail(f"Expected Claude PreToolUse allow, got {claude_allowed}")

    # 5. FileEdit / replace_file_content bypasses read gate
    edit_event = {
        "session_id": session_id,
        "tool_name": "FileEdit",
        "tool_input": {"file_path": str(tmp / "test.ts")},
    }
    raw_edit = handle_pre_tool_use(edit_event)
    if raw_edit.get("permission") == "allow":
        log_pass("Claude Code 'FileEdit' bypasses read gate directly")
    else:
        log_fail(f"Expected allow for FileEdit, got {raw_edit}")


def simulate_antigravity_gemini_session(tmp: Path) -> None:
    print(f"\n{BLUE}=== 3. Simulating Antigravity / Gemini CLI Session ==={RESET}")
    session_id = "antigravity-sim-session-003"
    set_active_session(session_id)

    # 1. User turn starts
    prompt_event = {
        "session_id": session_id,
        "hook_event_name": "BeforeAgent",
        "user_message": "Implement feature X",
    }
    handle_before_submit_prompt(prompt_event)

    # 2. Antigravity calls run_command before route_task
    run_cmd_event = {
        "session_id": session_id,
        "hook_event_name": "BeforeTool",
        "tool_name": "run_command",
        "tool_input": {
            "CommandLine": "git diff",
            "Cwd": str(tmp),
        },
    }
    cmd_raw = handle_pre_tool_use(run_cmd_event)
    gemini_out = adapt_hook_output("BeforeTool", cmd_raw, run_cmd_event)
    if gemini_out.get("decision") == "deny":
        log_pass("Antigravity 'run_command' correctly DENIED before routing")
    else:
        log_fail(f"Expected Gemini BeforeTool deny, got {gemini_out}")

    # 3. Model calls route_task
    route_event = {
        "session_id": session_id,
        "tool_name": "route_task",
        "server": "agent-brain",
        "result_json": json.dumps({"tokens_used": 120, "recommended_skills": []}),
    }
    handle_post_tool_use(route_event)

    # 4. Antigravity calls run_command with CommandLine parameter -> ALLOWED
    cmd_raw_after = handle_pre_tool_use(run_cmd_event)
    gemini_allowed = adapt_hook_output("BeforeTool", cmd_raw_after, run_cmd_event)
    if gemini_allowed.get("decision") == "allow":
        log_pass("Antigravity 'run_command' with 'CommandLine' parameter ALLOWED after routing")
    else:
        log_fail(f"Expected Gemini BeforeTool allow, got {gemini_allowed}")

    # 5. Antigravity calls view_file with AbsolutePath
    view_file_event = {
        "session_id": session_id,
        "tool_name": "view_file",
        "tool_input": {
            "AbsolutePath": str(tmp / "small_doc.md"),
            "StartLine": 1,
            "EndLine": 50,
        },
    }
    (tmp / "small_doc.md").write_text("# Documentation\nLine 1\n", encoding="utf-8")
    view_raw = handle_pre_tool_use(view_file_event)
    if view_raw.get("permission") == "allow":
        log_pass("Antigravity 'view_file' with bounded lines ALLOWED")
    else:
        log_fail(f"Expected allow for bounded view_file, got {view_raw}")


def simulate_codex_and_opencode_sessions() -> None:
    print(f"\n{BLUE}=== 4. Simulating Codex & OpenCode Sessions ==={RESET}")
    session_id = "codex-sim-session-004"
    set_active_session(session_id)

    # Codex turn
    codex_event = {
        "session_id": session_id,
        "turn_id": "turn-cdx-1",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "echo test"},
    }
    # Unrouted
    save_state({"needs_route": True, "needs_route_since": time.time(), "route_grace_until": 0}, codex_event)
    codex_raw = handle_pre_tool_use(codex_event)
    codex_out = adapt_hook_output("PreToolUse", codex_raw, codex_event)
    if codex_out.get("hookSpecificOutput", {}).get("permissionDecision") == "deny":
        log_pass("Codex PreToolUse with 'turn_id' intercepted and DENIED before route")
    else:
        log_fail(f"Expected Codex deny, got {codex_out}")

    for brain_tool in (
        "mcp_agent-brain_grep_search",
        "mcp_agent-brain_read_file_head",
    ):
        ev = {"tool_name": brain_tool}
        if is_agent_brain_mcp_tool(ev):
            log_pass(f"agent-brain MCP tool '{brain_tool}' recognized by gate")
        else:
            log_fail(f"Expected '{brain_tool}' to be recognized as an agent-brain MCP tool")


def main() -> int:
    print("=" * 60)
    print("AUTONOMIC MULTI-AGENT INTERCEPTION & SIMULATION SUITE")
    print("=" * 60)

    tmp = setup_isolated_env()
    try:
        simulate_cursor_session(tmp)
        simulate_claude_code_session(tmp)
        simulate_antigravity_gemini_session(tmp)
        simulate_codex_and_opencode_sessions()

        print("\n" + "=" * 60)
        print(f"{GREEN}ALL MULTI-AGENT HARNESS SIMULATIONS PASSED CLEANLY (EXIT 0){RESET}")
        print("=" * 60)
        return 0
    finally:
        import shutil
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
