#!/usr/bin/env python3
"""Cursor hook: require agent-brain route_task before other tools each user turn."""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

STATE_PATH = (
    Path(os.environ.get("AGENT_BRAIN_HOME", Path.home() / ".agent_brain"))
    / "hooks"
    / "route_state.json"
)

ROUTE_TOOL_NAMES = {
    "route_task",
    "MCP:route_task",
    "mcp:route_task",
    "mcp_agent-brain_route_task",
}

GRACE_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_GRACE_SECS", "120"))
STALE_ROUTE_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_STALE_SECS", "45"))


def disabled() -> bool:
    v = os.environ.get("AGENT_BRAIN_ROUTE_HOOKS", "1").strip().lower()
    return v in {"0", "false", "no", "off"}


def load_state() -> dict:
    if not STATE_PATH.exists():
        return {}
    try:
        return json.loads(STATE_PATH.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return {}


def save_state(state: dict) -> None:
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    STATE_PATH.write_text(json.dumps(state), encoding="utf-8")


def is_agent_brain_command(event: dict) -> bool:
    cmd = str(event.get("command") or "")
    server = str(event.get("server") or "")
    url = str(event.get("url") or "")
    return (
        "agent-brain" in cmd
        or server == "agent-brain"
        or "agent-brain" in url
    )


def is_route_task(event: dict) -> bool:
    tool = str(event.get("tool_name") or "").strip()
    if not tool:
        return False
    tool_lower = tool.lower()
    if tool in ROUTE_TOOL_NAMES:
        return True
    if tool_lower.endswith(":route_task") or tool_lower.endswith("_route_task"):
        return True
    # Cursor Agent tools: mcp_<server>_route_task
    if "route_task" in tool_lower and (
        "agent-brain" in tool_lower or "agent_brain" in tool_lower
    ):
        return True
    if tool == "route_task" and is_agent_brain_command(event):
        return True
    return False


def is_agent_brain_route_event(event: dict) -> bool:
    if not is_route_task(event):
        return False
    tool_lower = str(event.get("tool_name") or "").lower()
    return (
        is_agent_brain_command(event)
        or "agent-brain" in tool_lower
        or "agent_brain" in tool_lower
        or str(event.get("tool_name") or "") in ROUTE_TOOL_NAMES
    )


def parse_json_value(raw: object) -> object | None:
    if raw is None:
        return None
    if isinstance(raw, dict):
        return raw
    if isinstance(raw, list):
        return raw
    if isinstance(raw, str):
        text = raw.strip()
        if not text:
            return None
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return None
    return None


def unwrap_mcp_payload(data: object) -> dict | None:
    parsed = parse_json_value(data)
    if not isinstance(parsed, dict):
        return None

    # MCP CallToolResult: { "content": [ { "type": "text", "text": "{...}" } ] }
    content = parsed.get("content")
    if isinstance(content, list):
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "text":
                inner = parse_json_value(block.get("text"))
                if isinstance(inner, dict):
                    return inner

    if any(
        key in parsed
        for key in (
            "recommended_skills",
            "recommended_agents",
            "applicable_rules",
            "relevant_memory",
            "tokens_used",
        )
    ):
        return parsed
    return None


def route_response_useful(event: dict) -> bool:
    for key in (
        "result_json",
        "tool_result",
        "tool_output",
        "result",
        "output",
        "response",
    ):
        payload = unwrap_mcp_payload(event.get(key))
        if payload is None:
            continue
        if int(payload.get("tokens_used") or 0) > 0:
            return True
        for field in (
            "recommended_skills",
            "recommended_agents",
            "applicable_rules",
            "relevant_memory",
        ):
            value = payload.get(field)
            if isinstance(value, list) and value:
                return True
    return False


def deny_payload() -> dict:
    return {
        "permission": "deny",
        "agent_message": (
            "You must call agent-brain MCP tool route_task first with the user's "
            "message, current_working_directory, and open_files. If the response "
            "is empty (tokens_used 0), restart the agent-brain MCP server and "
            "retry route_task; pass explicit limits if needed."
        ),
        "user_message": "agent-brain hook: call route_task before other tools.",
    }


def route_attempt_failed(event: dict) -> bool:
    if not is_agent_brain_route_event(event):
        return False
    if event.get("success") is False:
        return True
    if event.get("error"):
        return True
    for key in ("errorMessage", "message"):
        err = str(event.get(key) or "").lower()
        if any(
            token in err
            for token in ("connection closed", "not connected", "mcp error")
        ):
            return True
    return False


def enter_grace(state: dict | None = None, seconds: float | None = None) -> None:
    state = state if state is not None else load_state()
    secs = seconds if seconds is not None else GRACE_SECS
    state["route_grace_until"] = time.time() + secs
    state["needs_route"] = False
    state.pop("needs_route_since", None)
    save_state(state)


def in_grace_period(state: dict) -> bool:
    until = state.get("route_grace_until")
    if not isinstance(until, (int, float)) or until <= 0:
        return False
    return time.time() < until


def stale_needs_route(state: dict) -> bool:
    if not state.get("needs_route"):
        return False
    since = state.get("needs_route_since")
    if not isinstance(since, (int, float)) or since <= 0:
        return False
    return (time.time() - since) >= STALE_ROUTE_SECS


def should_allow_without_route(state: dict) -> bool:
    return in_grace_period(state) or stale_needs_route(state)


def grace_allow_payload(state: dict) -> dict:
    reason = (
        "grace period after route_task failure"
        if in_grace_period(state)
        else "stale gate timeout"
    )
    return {
        "permission": "allow",
        "agent_message": (
            f"agent-brain route gate: proceeding without route_task ({reason}). "
            "Call route_task when MCP is available."
        ),
    }


def try_clear_route_gate(event: dict) -> None:
    if not is_agent_brain_route_event(event):
        return
    if event.get("success") is False or event.get("error"):
        return
    if not route_response_useful(event):
        return
    state = load_state()
    state["needs_route"] = False
    state.pop("needs_route_since", None)
    state["route_grace_until"] = 0
    if event.get("generation_id"):
        state["generation_id"] = event["generation_id"]
    save_state(state)


def handle_route_outcome(event: dict) -> None:
    if not is_agent_brain_route_event(event):
        return
    if route_attempt_failed(event):
        enter_grace()
        return
    try_clear_route_gate(event)


def handle_before_submit_prompt(_event: dict) -> dict:
    save_state(
        {
            "needs_route": True,
            "needs_route_since": time.time(),
            "route_grace_until": 0,
        }
    )
    return {"continue": True}


def handle_after_mcp_execution(event: dict) -> dict:
    handle_route_outcome(event)
    return {}


def handle_post_tool_use(event: dict) -> dict:
    # Cursor Agent MCP tools (mcp_agent-brain_*) clear the gate via postToolUse,
    # not afterMCPExecution.
    handle_route_outcome(event)
    return {}


def handle_pre_tool_use(event: dict) -> dict:
    if is_route_task(event):
        return {"permission": "allow"}
    state = load_state()
    if state.get("needs_route"):
        if should_allow_without_route(state):
            return grace_allow_payload(state)
        return deny_payload()
    return {"permission": "allow"}


def handle_before_mcp_execution(event: dict) -> dict:
    if is_route_task(event):
        return {"permission": "allow"}
    state = load_state()
    if state.get("needs_route"):
        if should_allow_without_route(state):
            return grace_allow_payload(state)
        return deny_payload()
    return {"permission": "allow"}


def main() -> int:
    if disabled():
        event_name = ""
        try:
            event = json.load(sys.stdin)
            event_name = event.get("hook_event_name", "")
        except json.JSONDecodeError:
            event = {}
        if event_name == "beforeSubmitPrompt":
            print(json.dumps({"continue": True}))
        elif event_name in {"preToolUse", "beforeMCPExecution", "beforeShellExecution"}:
            print(json.dumps({"permission": "allow"}))
        else:
            print("{}")
        return 0

    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        print(json.dumps({"permission": "allow"}))
        return 0

    name = event.get("hook_event_name", "")

    if name == "beforeSubmitPrompt":
        out = handle_before_submit_prompt(event)
    elif name == "afterMCPExecution":
        out = handle_after_mcp_execution(event)
    elif name == "postToolUse":
        out = handle_post_tool_use(event)
    elif name == "preToolUse":
        out = handle_pre_tool_use(event)
    elif name == "beforeMCPExecution":
        out = handle_before_mcp_execution(event)
    else:
        out = {}

    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
