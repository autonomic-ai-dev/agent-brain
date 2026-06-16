#!/usr/bin/env python3
"""Unit tests for route_gate hook helpers."""

from __future__ import annotations

import json
import tempfile
import time
import unittest
from pathlib import Path

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
    is_route_task,
    load_state,
    route_response_useful,
    should_gate_tool,
    stale_needs_route,
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
        out = handle_pre_tool_use({"tool_name": "mcp_agent-brain_store_memory"})
        self.assertEqual(out.get("permission"), "deny")

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
        self.assertFalse(should_gate_tool({"tool_name": "Shell"}))
        self.assertTrue(
            should_gate_tool({"tool_name": "mcp_agent-brain_store_memory"})
        )

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


if __name__ == "__main__":
    unittest.main()
