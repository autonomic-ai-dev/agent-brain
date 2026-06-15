#!/usr/bin/env python3
"""Unit tests for route_gate hook helpers."""

from __future__ import annotations

import json
import unittest

from route_gate import (
    is_route_task,
    route_response_useful,
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


if __name__ == "__main__":
    unittest.main()
