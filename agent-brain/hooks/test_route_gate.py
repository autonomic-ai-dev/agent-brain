#!/usr/bin/env python3
"""Unit tests for route_gate hook helpers."""

from __future__ import annotations

import json
import os
import shutil
import tempfile
import time
import unittest
from pathlib import Path

_ISOLATED_HOME = tempfile.TemporaryDirectory(prefix="agent-brain-route-test-")
os.environ["AGENT_BRAIN_HOME"] = _ISOLATED_HOME.name
for _session_var in (
    "CURSOR_CONVERSATION_ID",
    "AGENT_BRAIN_HOOK_SESSION",
    "AGY_CONVERSATION_ID",
    "CODEX_THREAD_ID",
):
    os.environ.pop(_session_var, None)

_ORIGINAL_PATH = os.environ.get("PATH", "")
_STUB_BIN = tempfile.TemporaryDirectory(prefix="agent-brain-stub-bin-")
_STUB_BRAIN = os.path.join(_STUB_BIN.name, "agent-brain")
with open(_STUB_BRAIN, "w", encoding="utf-8") as _stub:
    _stub.write("#!/bin/sh\nexit 0\n")
os.chmod(_STUB_BRAIN, 0o755)
os.environ["PATH"] = _STUB_BIN.name + os.pathsep + _ORIGINAL_PATH

import route_gate
from route_gate import (
    GATE_SCOPE,
    OFFLINE_SECS,
    STALE_ROUTE_SECS,
    enter_grace,
    enter_mcp_offline,
    handle_before_mcp_execution,
    handle_before_submit_prompt,
    handle_post_tool_use,
    handle_pre_tool_use,
    in_grace_period,
    in_mcp_offline,
    is_agent_brain_mcp_tool,
    is_route_task,
    check_read_tool_gate,
    load_state,
    route_response_useful,
    should_gate_tool,
    stale_needs_route,
    looks_like_verification,
    maybe_record_verify_outcome,
    session_id_from_event,
    set_active_session,
    is_cursor_edit_tool,
    record_scouted_path,
    path_is_scouted,
    maybe_record_scout_from_event,
    check_python_shell_policy,
    handle_before_shell_execution,
)


class RouteGateTests(unittest.TestCase):
    def test_post_tool_use_clears_gate(self) -> None:
        from route_gate import STATE_PATH, load_state, try_clear_route_gate

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        STATE_PATH.write_text('{"needs_route": true}', encoding="utf-8")
        payload = {
            "recommended_skills": [{"name": "rust-testing"}],
            "tokens_used": 120,
        }
        try_clear_route_gate(
            {
                "tool_name": "mcp_agent-brain_route_task",
                "tool_output": json.dumps(payload),
            }
        )
        self.assertFalse(load_state().get("needs_route"))

    def test_post_tool_use_clears_gate_with_list_shaped_tool_response(self) -> None:
        from route_gate import STATE_PATH, handle_post_tool_use, load_state, save_state, set_active_session

        set_active_session("global")
        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        save_state({"needs_route": True})
        inner_payload = {
            "recommended_skills": [{"name": "rust-testing"}],
            "tokens_used": 120,
        }
        # Claude Code returns a list of content blocks in tool_response / tool_result
        list_response = [{"type": "text", "text": json.dumps(inner_payload)}]
        handle_post_tool_use(
            {
                "hook_event_name": "PostToolUse",
                "tool_name": "mcp__agent-brain__route_task",
                "tool_response": list_response,
            }
        )
        self.assertFalse(load_state().get("needs_route"))

    def test_cursor_mcp_tool_names(self) -> None:
        for name in (
            "mcp_agent-brain_route_task",
            "MCP:route_task",
            "route_task",
        ):
            self.assertTrue(
                is_route_task({"tool_name": name}),
                f"expected route_task match for {name}",
            )

    def test_non_route_tools_rejected(self) -> None:
        self.assertFalse(is_route_task({"tool_name": "Shell"}))
        self.assertFalse(is_route_task({"tool_name": "mcp_agent-brain_store_memory"}))

    def test_useful_route_payload(self) -> None:
        payload = {
            "recommended_skills": [{"name": "rust-testing"}],
            "tokens_used": 120,
        }
        event = {"result_json": json.dumps(payload)}
        self.assertTrue(route_response_useful(event))

    def test_empty_route_payload_not_useful(self) -> None:
        payload = {
            "recommended_skills": [],
            "recommended_agents": [],
            "applicable_rules": [],
            "relevant_memory": [],
            "tokens_used": 0,
        }
        event = {"result_json": json.dumps(payload)}
        self.assertFalse(route_response_useful(event))

    def test_mcp_content_wrapper(self) -> None:
        inner = {"tokens_used": 50, "recommended_agents": [{"name": "reviewer"}]}
        wrapped = {
            "content": [{"type": "text", "text": json.dumps(inner)}],
        }
        event = {"result_json": json.dumps(wrapped)}
        self.assertTrue(route_response_useful(event))

    def test_failed_route_enters_grace(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        STATE_PATH.write_text('{"needs_route": true}', encoding="utf-8")
        handle_post_tool_use(
            {
                "tool_name": "mcp_agent-brain_route_task",
                "success": False,
                "errorMessage": "empty payload",
            }
        )
        state = load_state()
        self.assertFalse(state.get("needs_route"))
        self.assertTrue(in_grace_period(state))

    def test_disconnect_enters_offline(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        STATE_PATH.write_text('{"needs_route": true}', encoding="utf-8")
        handle_post_tool_use(
            {
                "tool_name": "mcp_agent-brain_route_task",
                "success": False,
                "errorMessage": "Not connected",
            }
        )
        state = load_state()
        self.assertTrue(in_mcp_offline(state))

    def test_grace_allows_other_tools(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        enter_grace({"needs_route": True, "needs_route_since": time.time()})
        out = handle_pre_tool_use({"tool_name": "Shell"})
        self.assertEqual(out.get("permission"), "allow")

    def test_stale_gate_allows_tools(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        state = {
            "needs_route": True,
            "needs_route_since": time.time() - STALE_ROUTE_SECS - 1,
        }
        STATE_PATH.write_text(json.dumps(state), encoding="utf-8")
        self.assertTrue(stale_needs_route(load_state()))
        out = handle_before_mcp_execution({"tool_name": "Shell"})
        self.assertEqual(out.get("permission"), "allow")

    def test_new_prompt_resets_grace(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        enter_grace()
        handle_before_submit_prompt({})
        state = load_state()
        self.assertTrue(state.get("needs_route"))
        self.assertEqual(state.get("route_grace_until"), 0)

    def test_scoped_gate_allows_shell_when_needs_route(self) -> None:
        from route_gate import STATE_PATH

        if GATE_SCOPE != "brain_mcp":
            self.skipTest("scoped gate test requires brain_mcp scope")
        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        STATE_PATH.write_text(
            json.dumps({"needs_route": True, "needs_route_since": time.time()}),
            encoding="utf-8",
        )
        out = handle_pre_tool_use({"tool_name": "Shell"})
        self.assertEqual(out.get("permission"), "allow")

    def test_scoped_gate_blocks_brain_tools_when_needs_route(self) -> None:
        from route_gate import STATE_PATH

        if GATE_SCOPE != "brain_mcp":
            self.skipTest("scoped gate test requires brain_mcp scope")
        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        STATE_PATH.write_text(
            json.dumps({"needs_route": True, "needs_route_since": time.time()}),
            encoding="utf-8",
        )
        for tool_name in (
            "mcp_agent-brain_store_memory",
            "mcp__agent-brain__store_memory",
            "agent-brain_store_memory",
            "agent_brain_grep_search",
        ):
            out = handle_pre_tool_use({"tool_name": tool_name})
            self.assertEqual(out.get("permission"), "deny", tool_name)

    def test_opencode_route_task_name(self) -> None:
        self.assertTrue(is_route_task({"tool_name": "agent-brain_route_task"}))
        self.assertTrue(
            is_agent_brain_mcp_tool({"tool_name": "agent-brain_store_memory"}),
        )
        self.assertFalse(
            is_agent_brain_mcp_tool({"tool_name": "agent-brain_route_task"}),
        )

    def test_claude_route_task_name(self) -> None:
        self.assertTrue(
            is_route_task({"tool_name": "mcp__agent-brain__route_task"}),
        )

    def test_offline_skips_new_prompt_gate(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        enter_mcp_offline()
        handle_before_submit_prompt({})
        state = load_state()
        self.assertFalse(state.get("needs_route"))
        self.assertTrue(in_mcp_offline(state))

    def test_offline_allows_shell(self) -> None:
        from route_gate import STATE_PATH

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        enter_mcp_offline({"needs_route": True, "needs_route_since": time.time()})
        out = handle_pre_tool_use({"tool_name": "Shell"})
        self.assertEqual(out.get("permission"), "allow")

    def test_should_gate_tool_brain_mcp(self) -> None:
        import route_gate

        orig = route_gate.GATE_SCOPE
        try:
            route_gate.GATE_SCOPE = "brain_mcp"
            self.assertFalse(should_gate_tool({"tool_name": "Shell"}))
            self.assertTrue(
                should_gate_tool({"tool_name": "mcp_agent-brain_store_memory"})
            )

            route_gate.GATE_SCOPE = "all"
            self.assertTrue(should_gate_tool({"tool_name": "Shell"}))
            self.assertTrue(
                should_gate_tool({"tool_name": "mcp_agent-brain_store_memory"})
            )
        finally:
            route_gate.GATE_SCOPE = orig

    def _hook_latency_p95(self, handler, event: dict, iterations: int = 500) -> float:
        import route_gate

        samples: list[float] = []
        for _ in range(iterations):
            start = time.perf_counter()
            handler(event)
            samples.append((time.perf_counter() - start) * 1000.0)
        samples.sort()
        idx = max(0, int(len(samples) * 0.95) - 1)
        return samples[idx]

    def test_hook_pre_tool_use_allow_p95_under_1ms(self) -> None:
        import route_gate

        old_path = route_gate.STATE_PATH
        try:
            with tempfile.TemporaryDirectory() as tmp:
                route_gate.STATE_PATH = Path(tmp) / "route_state.json"
                route_gate.STATE_PATH.write_text(
                    json.dumps({"needs_route": False}), encoding="utf-8"
                )
                p95 = self._hook_latency_p95(
                    handle_pre_tool_use, {"tool_name": "Shell"}
                )
                self.assertLess(p95, 1.0, f"allow-path hook p95 {p95:.3f}ms")
        finally:
            route_gate.STATE_PATH = old_path

    def test_hook_pre_tool_use_deny_p95_under_1ms(self) -> None:
        import route_gate

        old_path = route_gate.STATE_PATH
        try:
            with tempfile.TemporaryDirectory() as tmp:
                route_gate.STATE_PATH = Path(tmp) / "route_state.json"
                route_gate.STATE_PATH.write_text(
                    json.dumps(
                        {
                            "needs_route": True,
                            "needs_route_since": time.time(),
                        }
                    ),
                    encoding="utf-8",
                )
                p95 = self._hook_latency_p95(
                    handle_pre_tool_use, {"tool_name": "mcp_agent-brain_store_memory"}
                )
                self.assertLess(p95, 1.0, f"deny-path hook p95 {p95:.3f}ms")
        finally:
            route_gate.STATE_PATH = old_path


class ReadGateTests(unittest.TestCase):
    def test_denies_large_read_in_hard_mode(self) -> None:
        old_mode = route_gate.READ_GATE_MODE
        route_gate.READ_GATE_MODE = "hard"
        try:
            with tempfile.TemporaryDirectory() as tmp:
                route_gate.STATE_PATH = Path(tmp) / "route_state.json"
                route_gate.TOOL_EVENTS_PATH = Path(tmp) / "tool_events.jsonl"
                route_gate.STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
                route_gate.STATE_PATH.write_text(
                    json.dumps(
                        {
                            "suggested_native_tools": [{"tool": "grep_search"}],
                        }
                    ),
                    encoding="utf-8",
                )
                big = Path(tmp) / "big.log"
                big.write_text("x" * 70000, encoding="utf-8")
                result = check_read_tool_gate(
                    {
                        "tool_name": "Read",
                        "tool_input": json.dumps({"path": str(big)}),
                    }
                )
                self.assertIsNotNone(result)
                assert result is not None
                self.assertEqual(result["permission"], "deny")
        finally:
            route_gate.READ_GATE_MODE = old_mode

    def test_steers_large_read(self) -> None:
        old_mode = route_gate.READ_GATE_MODE
        route_gate.READ_GATE_MODE = "steer"
        try:
            with tempfile.TemporaryDirectory() as tmp:
                route_gate.STATE_PATH = Path(tmp) / "route_state.json"
                route_gate.TOOL_EVENTS_PATH = Path(tmp) / "tool_events.jsonl"
                route_gate.STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
                route_gate.STATE_PATH.write_text(
                    json.dumps(
                        {
                            "must_apply": [{"topic": "no-read-dist", "text": "never"}],
                            "suggested_native_tools": [{"tool": "grep_search"}],
                        }
                    ),
                    encoding="utf-8",
                )
                big = Path(tmp) / "big.log"
                big.write_text("x" * 70000, encoding="utf-8")
                result = check_read_tool_gate(
                    {
                        "tool_name": "Read",
                        "tool_input": json.dumps({"path": str(big)}),
                    }
                )
                self.assertIsNotNone(result)
                assert result is not None
                self.assertEqual(result["permission"], "allow")
                state = load_state()
                self.assertIn("anti_pattern_suggestion", state)
                self.assertEqual(state["anti_pattern_suggestion"].get("path"), str(big))
        finally:
            route_gate.READ_GATE_MODE = old_mode


class MultiHostHookOutputTests(unittest.TestCase):
    def test_gemini_before_tool_deny(self) -> None:
        out = route_gate.adapt_hook_output(
            "BeforeTool",
            {"permission": "deny", "agent_message": "call route_task first"},
        )
        self.assertEqual(out["decision"], "deny")
        self.assertIn("route_task", out["reason"])

    def test_claude_pre_tool_deny(self) -> None:
        out = route_gate.adapt_hook_output(
            "PreToolUse",
            {"permission": "deny", "agent_message": "call route_task first"},
            {"tool_name": "mcp__agent-brain__store_memory"},
        )
        self.assertEqual(out["hookSpecificOutput"]["permissionDecision"], "deny")
        self.assertEqual(
            out["hookSpecificOutput"]["permissionDecisionReason"], "call route_task first"
        )

    def test_claude_user_prompt_submit_allow(self) -> None:
        out = route_gate.adapt_hook_output(
            "UserPromptSubmit",
            {"continue": True},
        )
        self.assertEqual(out, {})

    def test_codex_pre_tool_use_allow_empty(self) -> None:
        out = route_gate.adapt_hook_output(
            "PreToolUse",
            {"permission": "allow"},
            {"turn_id": "turn-1", "tool_name": "mcp__agent-brain__read_file_head"},
        )
        self.assertEqual(out, {})

    def test_codex_pre_tool_use_allow_steers_with_context(self) -> None:
        out = route_gate.adapt_hook_output(
            "PreToolUse",
            {"permission": "allow", "agent_message": "prefer bounded reads"},
            {"turn_id": "turn-1"},
        )
        self.assertEqual(
            out["hookSpecificOutput"]["additionalContext"], "prefer bounded reads"
        )
        self.assertNotIn("permissionDecision", out["hookSpecificOutput"])

    def test_cursor_pre_tool_use_allow_passthrough(self) -> None:
        payload = {"permission": "allow", "agent_message": "ok"}
        out = route_gate.adapt_hook_output("preToolUse", payload)
        self.assertEqual(out, payload)


class StatefulWorkflowHookTests(unittest.TestCase):
    """LangGraph-inspired phase/state persistence across hook events."""

    def test_phase_persists_after_successful_route(self) -> None:
        import route_gate

        old_path = route_gate.STATE_PATH
        try:
            with tempfile.TemporaryDirectory() as tmp:
                route_gate.STATE_PATH = Path(tmp) / "route_state.json"
                route_gate.STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
                payload = {
                    "recommended_phase": "execution",
                    "recommended_agents": [{"name": "reviewer"}],
                    "tokens_used": 80,
                }
                route_gate.try_clear_route_gate(
                    {
                        "tool_name": "mcp_agent-brain_route_task",
                        "tool_output": json.dumps(payload),
                    }
                )
                state = load_state()
                self.assertFalse(state.get("needs_route"))
                self.assertEqual(state.get("route_phase"), "execution")
        finally:
            route_gate.STATE_PATH = old_path

    def test_new_user_prompt_resets_route_and_preserves_must_apply(self) -> None:
        import route_gate

        old_path = route_gate.STATE_PATH
        try:
            with tempfile.TemporaryDirectory() as tmp:
                route_gate.STATE_PATH = Path(tmp) / "route_state.json"
                route_gate.STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
                route_gate.STATE_PATH.write_text(
                    json.dumps(
                        {
                            "needs_route": False,
                            "route_phase": "planning",
                            "must_apply": [{"topic": "vitest", "text": "use vitest"}],
                        }
                    ),
                    encoding="utf-8",
                )
                handle_before_submit_prompt({})
                state = load_state()
                self.assertTrue(state.get("needs_route"))
                self.assertEqual(len(state.get("must_apply", [])), 1)
        finally:
            route_gate.STATE_PATH = old_path

    def test_codex_pre_tool_use_schema(self) -> None:
        out = route_gate.adapt_hook_output(
            "PreToolUse",
            {"permission": "deny", "agent_message": "route_task required"},
            {"turn_id": "turn-1"},
        )
        self.assertEqual(out["hookSpecificOutput"]["permissionDecision"], "deny")
        self.assertEqual(
            out["hookSpecificOutput"]["permissionDecisionReason"], "route_task required"
        )




class ScoutUnlockTests(unittest.TestCase):
    def test_edit_tools_bypass_read_gate(self) -> None:
        self.assertTrue(is_cursor_edit_tool({"tool_name": "StrReplace"}))
        self.assertTrue(is_cursor_edit_tool({"tool_name": "Write"}))
        self.assertIsNone(
            check_read_tool_gate(
                {
                    "tool_name": "StrReplace",
                    "tool_input": {"path": "/tmp/huge.rs", "old_string": "a", "new_string": "b"},
                }
            )
        )

    def test_scout_unlocks_large_native_read(self) -> None:
        from route_gate import STATE_PATH, READ_GATE_MODE
        import route_gate

        old = route_gate.READ_GATE_MODE
        route_gate.READ_GATE_MODE = "hard"
        try:
            STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
            if STATE_PATH.exists():
                STATE_PATH.unlink()
            big = Path(tempfile.mkdtemp()) / "big.rs"
            big.write_bytes(b"x" * (route_gate.READ_WARN_BYTES + 10))
            denied = check_read_tool_gate(
                {"tool_name": "Read", "tool_input": {"path": str(big)}}
            )
            self.assertIsNotNone(denied)
            self.assertEqual(denied.get("permission"), "deny")
            msg = (denied.get("agent_message") or "") + (denied.get("user_message") or "")
            self.assertIn("file_summary", msg)
            self.assertIn("StrReplace", msg)
            self.assertIn("shell+python", msg.lower())

            record_scouted_path(str(big), {"conversation_id": "scout-test"})
            allowed = check_read_tool_gate(
                {
                    "tool_name": "Read",
                    "tool_input": {"path": str(big)},
                    "conversation_id": "scout-test",
                }
            )
            self.assertIsNone(allowed)

            maybe_record_scout_from_event(
                {
                    "tool_name": "mcp_agent-brain_file_summary",
                    "success": True,
                    "tool_input": {"path": str(big)},
                    "conversation_id": "scout-test-2",
                }
            )
            self.assertTrue(
                path_is_scouted(
                    {"scouted_paths": {str(big.resolve()): time.time()}},
                    str(big),
                )
            )
        finally:
            route_gate.READ_GATE_MODE = old

    def test_python_shell_policy_denies_workarounds(self) -> None:
        denied_c = check_python_shell_policy("python3 -c \"print(open('f').read())\"")
        self.assertIsNotNone(denied_c)
        self.assertEqual(denied_c.get("permission"), "deny")
        denied_h = check_python_shell_policy("python3 <<'PY'\nprint(1)\nPY")
        self.assertIsNotNone(denied_h)
        denied_pipe = check_python_shell_policy("echo 'x' | python3")
        self.assertIsNotNone(denied_pipe)
        denied_stdin = check_python_shell_policy("python3 -")
        self.assertIsNotNone(denied_stdin)

    def test_python_shell_policy_allows_project_runs(self) -> None:
        self.assertIsNone(check_python_shell_policy("python3 -m pytest hooks/"))
        self.assertIsNone(check_python_shell_policy("python3 scripts/gen.py --out x"))
        self.assertIsNone(check_python_shell_policy("cargo test"))
        self.assertIsNone(check_python_shell_policy(None))
        # *.py paths / prose must not false-positive with cat <<EOF or "python -c" in -m text
        self.assertIsNone(
            check_python_shell_policy(
                "git add hooks/route_gate.py && git commit -m \"$(cat <<'EOF'\nmsg\nEOF\n)\""
            )
        )
        self.assertIsNone(
            check_python_shell_policy(
                'git commit -m "deny python -c workarounds at tool level"'
            )
        )

    def test_before_shell_execution_blocks_python_c(self) -> None:
        out = handle_before_shell_execution(
            {"hook_event_name": "beforeShellExecution", "command": "python3 -c 'print(1)'"}
        )
        self.assertEqual(out.get("permission"), "deny")
        allowed = handle_before_shell_execution(
            {
                "hook_event_name": "beforeShellExecution",
                "command": "python3 -m pytest hooks/test_route_gate.py",
            }
        )
        self.assertEqual(allowed.get("permission"), "allow")

    def test_pre_tool_use_shell_python_c_denied(self) -> None:
        out = handle_pre_tool_use(
            {
                "tool_name": "Shell",
                "tool_input": {"command": "python3 -c \"open('x').read()\""},
            }
        )
        self.assertEqual(out.get("permission"), "deny")
        self.assertIn("python -c", (out.get("user_message") or "").lower() + (out.get("agent_message") or "").lower())

    def test_python_deny_is_agent_agnostic_for_bash_and_before_tool(self) -> None:
        """Claude Bash PreToolUse + Gemini BeforeTool share route_gate.py policy."""
        from route_gate import adapt_hook_output, is_shell_tool

        self.assertTrue(is_shell_tool("Bash"))
        self.assertTrue(is_shell_tool("bash"))
        self.assertTrue(is_shell_tool("shell_command"))
        self.assertFalse(is_shell_tool("Write"))

        bash_denied = handle_pre_tool_use(
            {
                "tool_name": "Bash",
                "tool_input": {"command": "python3 -c 'print(1)'"},
            }
        )
        self.assertEqual(bash_denied.get("permission"), "deny")
        claude = adapt_hook_output("PreToolUse", bash_denied, {})
        self.assertEqual(
            claude.get("hookSpecificOutput", {}).get("permissionDecision"), "deny"
        )

        gemini_event = {
            "hook_event_name": "BeforeTool",
            "tool_name": "run_command",
            "tool_input": {"command": "python3 -c 'print(1)'"},
        }
        gemini_raw = handle_pre_tool_use(gemini_event)
        self.assertEqual(gemini_raw.get("permission"), "deny")
        gemini = adapt_hook_output("BeforeTool", gemini_raw, gemini_event)
        self.assertEqual(gemini.get("decision"), "deny")

    def test_before_shell_execution_denied_when_route_needed(self) -> None:
        from route_gate import STATE_PATH, save_state

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        save_state({"needs_route": True, "needs_route_since": time.time(), "route_grace_until": 0})
        res = handle_before_shell_execution({"command": "cargo test"})
        self.assertEqual(res.get("permission"), "deny")
        self.assertIn("route_task", res.get("agent_message", ""))

    def test_command_policy_allows_when_bash_gate_allows(self) -> None:
        import subprocess
        from unittest import mock

        from route_gate import check_command_policy

        allowed = subprocess.CompletedProcess(args=[], returncode=0, stdout="", stderr="")
        with mock.patch("route_gate.shutil.which", return_value="/bin/agent-brain"), mock.patch(
            "route_gate.subprocess.run", return_value=allowed
        ):
            self.assertIsNone(check_command_policy("cargo test"))

    def test_command_policy_warning_is_not_a_denial(self) -> None:
        import subprocess
        from unittest import mock

        from route_gate import check_command_policy

        warned = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=json.dumps({"systemMessage": "agent-brain warning: something"}),
            stderr="",
        )
        with mock.patch("route_gate.shutil.which", return_value="/bin/agent-brain"), mock.patch(
            "route_gate.subprocess.run", return_value=warned
        ):
            self.assertIsNone(check_command_policy("git reset --hard"))

    def test_command_policy_denies_when_bash_gate_blocks(self) -> None:
        import subprocess
        from unittest import mock

        from route_gate import check_command_policy

        reason = "agent-brain blocked this command: deletes the filesystem root."
        blocked = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=json.dumps(
                {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                }
            ),
            stderr="",
        )
        with mock.patch("route_gate.shutil.which", return_value="/bin/agent-brain"), mock.patch(
            "route_gate.subprocess.run", return_value=blocked
        ):
            denied = check_command_policy("rm -rf /")
        self.assertIsNotNone(denied)
        self.assertEqual(denied.get("permission"), "deny")
        self.assertEqual(denied.get("agent_message"), reason)

    def test_command_policy_timeout_fails_closed(self) -> None:
        import subprocess
        from unittest import mock

        from route_gate import check_command_policy

        with mock.patch("route_gate.shutil.which", return_value="/bin/agent-brain"), mock.patch(
            "route_gate.subprocess.run",
            side_effect=subprocess.TimeoutExpired("agent-brain", 2.0),
        ):
            denied = check_command_policy("echo hello")
        self.assertIsNotNone(denied)
        self.assertEqual(denied.get("permission"), "deny")
        self.assertIn("failing closed", denied.get("agent_message", ""))

    def test_command_policy_os_error_fails_closed(self) -> None:
        from unittest import mock

        from route_gate import check_command_policy

        with mock.patch("route_gate.shutil.which", return_value="/bin/agent-brain"), mock.patch(
            "route_gate.subprocess.run", side_effect=OSError("exec failed")
        ):
            denied = check_command_policy("echo hello")
        self.assertIsNotNone(denied)
        self.assertEqual(denied.get("permission"), "deny")

    def test_command_policy_fails_closed_without_agent_brain_binary(self) -> None:
        from unittest import mock

        from route_gate import check_command_policy

        with mock.patch("route_gate.shutil.which", return_value=None), mock.patch(
            "route_gate.subprocess.run"
        ) as run:
            denied = check_command_policy("cargo test")
        self.assertIsNotNone(denied)
        self.assertEqual(denied.get("permission"), "deny")
        self.assertIn("failing closed", denied.get("agent_message", ""))
        run.assert_not_called()

    @unittest.skipUnless(shutil.which("agent-brain", path=_ORIGINAL_PATH), "agent-brain not installed")
    def test_command_policy_real_binary_blocks_root_delete(self) -> None:
        from unittest import mock

        from route_gate import check_command_policy

        with mock.patch.dict(os.environ, {"PATH": _ORIGINAL_PATH}):
            denied = check_command_policy("rm -rf /")
        self.assertIsNotNone(denied)
        self.assertEqual(denied.get("permission"), "deny")

    def test_antigravity_gemini_extractors(self) -> None:
        from route_gate import extract_read_path, extract_shell_command

        read_ev = {"tool_name": "view_file", "tool_input": {"AbsolutePath": "/tmp/test.rs"}}
        self.assertEqual(extract_read_path(read_ev), "/tmp/test.rs")

        shell_ev = {"tool_name": "run_command", "tool_input": {"CommandLine": "git status"}}
        self.assertEqual(extract_shell_command(shell_ev), "git status")

    def test_agent_brain_mcp_matching_ignores_other_servers(self) -> None:
        from route_gate import is_agent_brain_mcp_tool

        self.assertTrue(is_agent_brain_mcp_tool({"tool_name": "mcp_agent-brain_grep_search"}))
        self.assertFalse(is_agent_brain_mcp_tool({"tool_name": "mcp_agent-body_muscle_execute_bash"}))
        self.assertFalse(is_agent_brain_mcp_tool({"tool_name": "eyes_describe_dom", "server": "agent-body"}))
        self.assertFalse(is_agent_brain_mcp_tool({"tool_name": "route_task"}))

    def test_before_shell_execution_allowed_when_routed(self) -> None:
        from route_gate import STATE_PATH, save_state

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        event = {"command": "cargo test"}
        save_state({"needs_route": False, "route_grace_until": 0}, event)
        res = handle_before_shell_execution(event)
        self.assertEqual(res.get("permission"), "allow")

    def test_claude_bash_pre_tool_use_denied_when_route_needed(self) -> None:
        from route_gate import STATE_PATH, save_state, adapt_hook_output

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        save_state({"needs_route": True, "needs_route_since": time.time(), "route_grace_until": 0})
        ev = {"tool_name": "Bash", "tool_input": {"command": "cargo test"}}
        raw = handle_pre_tool_use(ev)
        self.assertEqual(raw.get("permission"), "deny")
        claude = adapt_hook_output("PreToolUse", raw, ev)
        self.assertEqual(claude.get("hookSpecificOutput", {}).get("permissionDecision"), "deny")
        self.assertIn("route_task", claude.get("hookSpecificOutput", {}).get("permissionDecisionReason", ""))

    def test_edit_tools_bypass_read_gate(self) -> None:
        from route_gate import is_cursor_edit_tool, is_cursor_read_tool

        for edit_tool in ("Write", "StrReplace", "replace_file_content", "write_to_file", "FileEdit"):
            ev = {"tool_name": edit_tool}
            self.assertTrue(is_cursor_edit_tool(ev), f"Expected {edit_tool} to be an edit tool")
            self.assertFalse(is_cursor_read_tool(ev), f"Expected {edit_tool} NOT to be a read tool")

    def test_claude_and_antigravity_read_tools_detected(self) -> None:
        from route_gate import is_cursor_read_tool

        for read_tool in ("read", "read_file", "FileRead", "view", "view_file"):
            ev = {"tool_name": read_tool}
            self.assertTrue(is_cursor_read_tool(ev), f"Expected {read_tool} to be recognized as a read tool")


def test_looks_like_verification_allowlist():
    assert looks_like_verification("cargo test -p agent-brain")
    assert looks_like_verification("npm test")
    assert looks_like_verification("./.verify/scripts/doctor.sh")
    assert not looks_like_verification("agent-brain verify -- cargo test")
    assert not looks_like_verification("ls -la")


def test_maybe_record_verify_skips_non_shell(monkeypatch):
    called = []

    def fake_popen(*args, **kwargs):
        called.append((args, kwargs))
        class P:
            pass
        return P()

    monkeypatch.setattr("route_gate.subprocess.Popen", fake_popen)
    maybe_record_verify_outcome({"tool_name": "Write", "tool_input": {"path": "x"}})
    assert called == []
    maybe_record_verify_outcome(
        {
            "tool_name": "Shell",
            "success": True,
            "tool_input": {"command": "cargo test"},
        }
    )
    assert len(called) == 1
    argv = called[0][0][0]
    assert "verify" in argv and "record" in argv
    assert "--source" in argv and "hook" in argv


class SessionGateTests(unittest.TestCase):
    def test_session_isolation(self) -> None:
        from route_gate import STATE_PATH, load_state, save_state, set_active_session

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        if STATE_PATH.exists():
            STATE_PATH.unlink()
        set_active_session("conv-a")
        save_state({"needs_route": True, "needs_route_since": time.time()})
        set_active_session("conv-b")
        save_state({"needs_route": False})
        set_active_session("conv-a")
        self.assertTrue(load_state().get("needs_route"))
        set_active_session("conv-b")
        self.assertFalse(load_state().get("needs_route"))

    def test_before_submit_does_not_rearm_same_generation(self) -> None:
        from route_gate import STATE_PATH, load_state, set_active_session

        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        if STATE_PATH.exists():
            STATE_PATH.unlink()
        event = {"generation_id": "gen-1", "conversation_id": "c1"}
        handle_before_submit_prompt(event)
        self.assertTrue(load_state(event).get("needs_route"))
        # Simulate successful route
        set_active_session(session_id_from_event(event))
        st = load_state(event)
        st["needs_route"] = False
        st["routed_generation"] = "gen-1"
        from route_gate import save_state

        save_state(st, event)
        # Duplicate mid-turn submit for same generation must not re-arm
        handle_before_submit_prompt(event)
        self.assertFalse(load_state(event).get("needs_route"))
        # New generation must re-arm
        event2 = {"generation_id": "gen-2", "conversation_id": "c1"}
        handle_before_submit_prompt(event2)
        self.assertTrue(load_state(event2).get("needs_route"))

    def test_toolsearch_allowed_during_needs_route(self) -> None:
        from route_gate import (
            STATE_PATH,
            handle_pre_tool_use,
            save_state,
            load_state,
            is_meta_or_discovery_tool,
            set_active_session,
        )

        set_active_session("global")
        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        if STATE_PATH.exists():
            STATE_PATH.unlink()
        save_state({"needs_route": True, "needs_route_since": time.time()})
        for name in ("ToolSearch", "tool_search", "mcp_list_tools", "mcp__toolsearch"):
            self.assertTrue(is_meta_or_discovery_tool({"tool_name": name}))
            out = handle_pre_tool_use({"tool_name": name, "tool_input": {"query": "route_task"}})
            self.assertEqual(out.get("permission"), "allow", f"{name} should be allowed before route_task")
        # Ensure gate is still active for standard tools (not prematurely cleared)
        self.assertTrue(load_state().get("needs_route"))
        blocked = handle_pre_tool_use({"tool_name": "read_file", "tool_input": {"path": "src/main.rs"}})
        self.assertEqual(blocked.get("permission"), "deny")

    def test_claude_code_pre_tool_use_toolsearch_allowed(self) -> None:
        from route_gate import adapt_hook_output, handle_pre_tool_use, save_state, set_active_session

        set_active_session("global")
        save_state({"needs_route": True, "needs_route_since": time.time()})
        event = {
            "hook_event_name": "PreToolUse",
            "tool_name": "ToolSearch",
            "tool_input": {"query": "route_task"},
        }
        out = handle_pre_tool_use(event)
        adapted = adapt_hook_output("PreToolUse", out, event)
        self.assertEqual(
            adapted.get("hookSpecificOutput", {}).get("permissionDecision"),
            "allow",
        )

class SessionRouteReuseTests(unittest.TestCase):
    def setUp(self) -> None:
        from unittest import mock

        import route_gate

        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        patcher = mock.patch.object(
            route_gate, "STATE_PATH", Path(self._tmp.name) / "route_state.json"
        )
        patcher.start()
        self.addCleanup(patcher.stop)
        self.cwd = str(Path(self._tmp.name) / "repo")

    def _route_success(self, cwd: str) -> None:
        from route_gate import try_clear_route_gate

        try_clear_route_gate(
            {
                "tool_name": "mcp_agent-brain_route_task",
                "tool_output": json.dumps(
                    {"recommended_skills": [{"name": "rust-testing"}], "tokens_used": 120}
                ),
                "cwd": cwd,
            }
        )

    def _prompt(self, cwd: str) -> None:
        from route_gate import handle_before_submit_prompt

        handle_before_submit_prompt({"cwd": cwd, "prompt": "continue"})

    def test_first_prompt_of_a_session_requires_route(self) -> None:
        from route_gate import load_state

        self._prompt(self.cwd)

        self.assertTrue(load_state().get("needs_route"))

    def test_fresh_route_is_reused_on_later_prompts_in_same_directory(self) -> None:
        from route_gate import load_state

        self._route_success(self.cwd)
        self._prompt(self.cwd)
        self._prompt(self.cwd)

        state = load_state()
        self.assertFalse(state.get("needs_route"))
        self.assertIn("last_turn_ping", state)

    def test_route_expires_after_session_ttl(self) -> None:
        from unittest import mock

        import route_gate

        self._route_success(self.cwd)
        with mock.patch.object(route_gate, "SESSION_ROUTE_TTL_SECS", 0.0):
            self._prompt(self.cwd)

        self.assertTrue(route_gate.load_state().get("needs_route"))

    def test_changing_directory_requires_route(self) -> None:
        import route_gate

        self._route_success(self.cwd)
        self._prompt(str(Path(self._tmp.name) / "other-repo"))

        self.assertTrue(route_gate.load_state().get("needs_route"))

    def test_failed_route_is_not_reused(self) -> None:
        import route_gate

        self._route_success(self.cwd)
        state = route_gate.load_state()
        state["needs_route"] = True
        route_gate.save_state(state)
        self._prompt(self.cwd)

        after = route_gate.load_state()
        self.assertTrue(after.get("needs_route"))
        self.assertNotIn("last_turn_ping", after)

    def test_route_without_timestamp_is_not_reused(self) -> None:
        import route_gate

        state = route_gate.load_state()
        state["routed_cwd"] = self.cwd
        route_gate.save_state(state)
        self._prompt(self.cwd)

        self.assertTrue(route_gate.load_state().get("needs_route"))


class TestIsolationTests(unittest.TestCase):
    def test_hook_state_stays_inside_isolated_home(self) -> None:
        self.assertEqual(
            str(route_gate.default_brain_home()),
            _ISOLATED_HOME.name,
            "the default brain home must resolve to the isolated test home",
        )


if __name__ == "__main__":
    unittest.main()

