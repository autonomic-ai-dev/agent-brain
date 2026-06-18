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
OFFLINE_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_OFFLINE_SECS", "1800"))
# brain_mcp = gate only agent-brain MCP tools; all = gate every tool (legacy strict mode).
GATE_SCOPE = os.environ.get("AGENT_BRAIN_ROUTE_GATE_SCOPE", "brain_mcp").strip().lower()
READ_GATE_MODE = os.environ.get("AGENT_BRAIN_READ_GATE", "steer").strip().lower()
READ_WARN_BYTES = int(os.environ.get("AGENT_BRAIN_READ_WARN_BYTES", "65536"))
BLOCKED_PATH_SEGMENTS = ("dist", "node_modules", "target", "build", ".git", ".next", "coverage")
TOOL_EVENTS_PATH = STATE_PATH.parent / "tool_events.jsonl"


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


def is_agent_brain_mcp_tool(event: dict) -> bool:
    if is_route_task(event):
        return False
    tool = str(event.get("tool_name") or "").strip()
    tool_lower = tool.lower()
    if tool_lower.startswith("mcp_agent-brain_"):
        return True
    if tool_lower.startswith("mcp_agent_brain_"):
        return True
    if tool_lower.startswith("mcp_agent-brain-"):
        return True
    if tool_lower.startswith("mcp_agent_brain-"):
        return True
    if "agent-brain" in tool_lower and "mcp" in tool_lower:
        return True
    server = str(event.get("server") or "").strip().lower()
    if server in {"agent-brain", "agent_brain"}:
        return True
    return is_agent_brain_command(event)


def should_gate_tool(event: dict) -> bool:
    if GATE_SCOPE == "all":
        return True
    if GATE_SCOPE == "brain_mcp":
        return is_agent_brain_mcp_tool(event)
    return True


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
            "suggested_native_tools",
            "must_apply",
        )
    ):
        return parsed
    return None


def route_response_useful(event: dict) -> bool:
    for key in (
        "result_json",
        "tool_result",
        "tool_output",
        "tool_response",
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


def disconnect_error(event: dict) -> bool:
    for key in ("errorMessage", "message", "error"):
        err = str(event.get(key) or "").lower()
        if any(
            token in err
            for token in (
                "connection closed",
                "not connected",
                "mcp error",
                "tool execution error",
            )
        ):
            return True
    return False


def route_attempt_failed(event: dict) -> bool:
    if not is_agent_brain_route_event(event):
        return False
    if event.get("success") is False:
        return True
    if event.get("error"):
        return True
    return disconnect_error(event)


def enter_grace(state: dict | None = None, seconds: float | None = None) -> None:
    state = state if state is not None else load_state()
    secs = seconds if seconds is not None else GRACE_SECS
    state["route_grace_until"] = time.time() + secs
    state["needs_route"] = False
    state.pop("needs_route_since", None)
    save_state(state)


def enter_mcp_offline(state: dict | None = None, seconds: float | None = None) -> None:
    state = state if state is not None else load_state()
    secs = seconds if seconds is not None else OFFLINE_SECS
    state["mcp_offline_until"] = time.time() + secs
    state["needs_route"] = False
    state.pop("needs_route_since", None)
    state["route_grace_until"] = time.time() + secs
    save_state(state)


def clear_mcp_offline(state: dict) -> None:
    state.pop("mcp_offline_until", None)


def in_grace_period(state: dict) -> bool:
    until = state.get("route_grace_until")
    if not isinstance(until, (int, float)) or until <= 0:
        return False
    return time.time() < until


def in_mcp_offline(state: dict) -> bool:
    until = state.get("mcp_offline_until")
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
    return in_grace_period(state) or stale_needs_route(state) or in_mcp_offline(state)


def grace_allow_payload(state: dict) -> dict:
    if in_mcp_offline(state):
        reason = "MCP offline mode"
    elif in_grace_period(state):
        reason = "grace period after route_task failure"
    else:
        reason = "stale gate timeout"
    return {
        "permission": "allow",
        "agent_message": (
            f"agent-brain route gate: proceeding without route_task ({reason}). "
            "Call route_task when MCP is available; toggle agent-brain in "
            "Cursor Settings → MCP if it stays disconnected."
        ),
    }


def update_route_context(payload: dict) -> None:
    state = load_state()
    state["route_log_id"] = payload.get("log_id")
    state["route_phase"] = payload.get("recommended_phase")
    state["must_apply"] = payload.get("must_apply", [])
    state["suggested_native_tools"] = payload.get("suggested_native_tools", [])
    state["route_context_at"] = time.time()
    save_state(state)


def append_tool_event(record: dict) -> None:
    TOOL_EVENTS_PATH.parent.mkdir(parents=True, exist_ok=True)
    with TOOL_EVENTS_PATH.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record) + "\n")


def write_anti_pattern_suggestion(path: str, reason: str) -> None:
    state = load_state()
    topic = "no-read-dist" if "dist" in path.lower() else f"no-full-read-{Path(path).name}"
    state["anti_pattern_suggestion"] = {
        "topic": topic,
        "fact": f"Never read {path} whole — use grep_search, file_summary, read_file_head/tail.",
        "polarity": "negative",
        "path": path,
        "reason": reason,
        "suggested_at": time.time(),
        "apply_with": "store_memory",
    }
    save_state(state)


def is_cursor_read_tool(event: dict) -> bool:
    tool = str(event.get("tool_name") or "").strip().lower()
    return tool in {
        "read",
        "mcp_read",
        "read_file",
        "readfile",
        "readfiletool",
    } or tool.endswith("_read") or tool.startswith("read_")


def extract_read_path(event: dict) -> str | None:
    for key in ("tool_input", "arguments", "args", "input"):
        raw = event.get(key)
        parsed = parse_json_value(raw)
        if isinstance(parsed, dict):
            for field in ("path", "target_file", "file", "file_path"):
                value = parsed.get(field)
                if isinstance(value, str) and value.strip():
                    return value.strip()
    return None


def file_size_bytes(path: str) -> int | None:
    try:
        return Path(path).expanduser().resolve().stat().st_size
    except OSError:
        return None


def path_is_blocked(path: str) -> bool:
    lower = path.lower().replace("\\", "/")
    return any(f"/{seg}/" in lower or lower.endswith(f"/{seg}") or lower.startswith(f"{seg}/") for seg in BLOCKED_PATH_SEGMENTS)


def check_read_tool_gate(event: dict) -> dict | None:
    if READ_GATE_MODE == "off":
        return None
    if not is_cursor_read_tool(event):
        return None

    state = load_state()
    path = extract_read_path(event)
    must_apply = state.get("must_apply") or []
    suggested = state.get("suggested_native_tools") or []
    tool_hint = ", ".join(
        t.get("tool", "") for t in suggested if isinstance(t, dict) and t.get("tool")
    ) or "grep_search, file_summary, read_file_head"

    if path and path_is_blocked(path):
        write_anti_pattern_suggestion(path, "blocked path segment")
        append_tool_event(
            {
                "timestamp": int(time.time() * 1000),
                "tool_name": "cursor_read",
                "path": path,
                "tokens_used": 0,
                "must_apply_active": bool(must_apply),
                "phase": state.get("route_phase"),
            }
        )
        if READ_GATE_MODE == "hard":
            return {
                "permission": "deny",
                "agent_message": (
                    f"Read denied on blocked path `{path}`. Use agent-brain "
                    f"{tool_hint} instead, or set allow_blocked_paths with user approval."
                ),
                "user_message": "agent-brain: blocked Read on dist/node_modules/target/build.",
            }
        return {
            "permission": "allow",
            "agent_message": (
                f"Steer: `{path}` is blocked (dist/node_modules/target). Prefer agent-brain "
                f"{tool_hint}. store_memory anti-pattern staged — run store_memory if user agrees."
            ),
        }

    size = file_size_bytes(path) if path else None
    large = size is not None and size > READ_WARN_BYTES
    if must_apply or large:
        reason = "must_apply active" if must_apply else f"file > {READ_WARN_BYTES} bytes"
        if path:
            write_anti_pattern_suggestion(path, reason)
        append_tool_event(
            {
                "timestamp": int(time.time() * 1000),
                "tool_name": "cursor_read",
                "path": path,
                "tokens_used": max((size or 0) // 4, 1),
                "must_apply_active": bool(must_apply),
                "phase": state.get("route_phase"),
            }
        )
        if READ_GATE_MODE == "hard" and must_apply:
            return {
                "permission": "deny",
                "agent_message": (
                    f"Read denied while must_apply constraints are active. Use agent-brain "
                    f"{tool_hint} first."
                ),
                "user_message": "agent-brain: must_apply — use bounded reads.",
            }
        return {
            "permission": "allow",
            "agent_message": (
                f"Token steer: prefer agent-brain {tool_hint} before full Read"
                + (f" on `{path}`" if path else "")
                + ". Anti-pattern suggestion saved for store_memory."
            ),
        }
    return None


def try_clear_route_gate(event: dict) -> None:
    if not is_agent_brain_route_event(event):
        return
    if event.get("success") is False or event.get("error"):
        return
    if not route_response_useful(event):
        return
    for key in (
        "result_json",
        "tool_result",
        "tool_output",
        "tool_response",
        "result",
        "output",
        "response",
    ):
        payload = unwrap_mcp_payload(event.get(key))
        if payload is not None:
            update_route_context(payload)
            break
    state = load_state()
    state["needs_route"] = False
    state.pop("needs_route_since", None)
    state["route_grace_until"] = 0
    clear_mcp_offline(state)
    if event.get("generation_id"):
        state["generation_id"] = event["generation_id"]
    save_state(state)


def handle_route_outcome(event: dict) -> None:
    if not is_agent_brain_route_event(event):
        return
    if route_attempt_failed(event):
        if disconnect_error(event):
            enter_mcp_offline()
        else:
            enter_grace()
        return
    try_clear_route_gate(event)


def handle_before_submit_prompt(_event: dict) -> dict:
    state = load_state()
    if in_mcp_offline(state):
        save_state(state)
        return {"continue": True}
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


def gate_tool_use(event: dict) -> dict:
    if is_route_task(event):
        return {"permission": "allow"}
    if not should_gate_tool(event):
        return {"permission": "allow"}
    state = load_state()
    if state.get("needs_route"):
        if should_allow_without_route(state):
            return grace_allow_payload(state)
        return deny_payload()
    return {"permission": "allow"}


def handle_pre_tool_use(event: dict) -> dict:
    read_gate = check_read_tool_gate(event)
    if read_gate is not None:
        return read_gate
    return gate_tool_use(event)


def handle_before_mcp_execution(event: dict) -> dict:
    return gate_tool_use(event)


def adapt_hook_output(event_name: str, out: dict) -> dict:
    """Map Cursor-style permission payloads to Claude Code / Gemini hook schemas."""
    if not out:
        return out
    permission = out.get("permission")
    agent_message = out.get("agent_message") or out.get("user_message") or ""
    if event_name in {"BeforeTool", "BeforeAgent", "BeforeModel"}:
        if permission == "deny":
            return {"decision": "deny", "reason": agent_message}
        if permission == "allow" and agent_message:
            return {"decision": "allow", "systemMessage": agent_message}
        return {"decision": "allow"}
    if event_name in {"PreToolUse", "beforeMCPExecution", "beforeShellExecution"}:
        if permission == "deny":
            return {
                "hookSpecificOutput": {"permissionDecision": "deny"},
                "systemMessage": agent_message,
            }
        if permission == "allow" and agent_message:
            return {
                "hookSpecificOutput": {"permissionDecision": "allow"},
                "systemMessage": agent_message,
            }
        return {"hookSpecificOutput": {"permissionDecision": "allow"}}
    if event_name in {"UserPromptSubmit", "BeforeAgent"}:
        if out.get("continue") is False:
            return {"decision": "deny", "reason": agent_message}
        return {"continue": out.get("continue", True)}
    return out


def normalize_event_name(name: str) -> str:
    aliases = {
        "UserPromptSubmit": "beforeSubmitPrompt",
        "BeforeAgent": "beforeSubmitPrompt",
        "PreToolUse": "preToolUse",
        "BeforeTool": "preToolUse",
        "PostToolUse": "postToolUse",
        "AfterTool": "postToolUse",
        "beforeMCPExecution": "beforeMCPExecution",
    }
    return aliases.get(name, name)


def normalize_tool_event(event: dict) -> dict:
    """Copy Gemini/Claude field names onto Cursor-style keys used by gate helpers."""
    out = dict(event)
    if "tool_name" not in out and isinstance(event.get("tool_name"), str):
        out["tool_name"] = event["tool_name"]
    if "tool_name" not in out:
        for key in ("tool", "name"):
            if isinstance(event.get(key), str):
                out["tool_name"] = event[key]
                break
    if "tool_input" not in out:
        for key in ("tool_input", "arguments", "args", "input"):
            if key in event:
                out["tool_input"] = event[key]
                break
    if "hook_event_name" not in out:
        out["hook_event_name"] = event.get("hook_event_name", "")
    return out


def main() -> int:
    if disabled():
        event_name = ""
        try:
            event = json.load(sys.stdin)
            event_name = event.get("hook_event_name", "")
        except json.JSONDecodeError:
            event = {}
        if event_name in {"beforeSubmitPrompt", "UserPromptSubmit", "BeforeAgent"}:
            print(json.dumps({"continue": True}))
        elif event_name in {
            "preToolUse",
            "PreToolUse",
            "BeforeTool",
            "beforeMCPExecution",
            "beforeShellExecution",
        }:
            print(json.dumps(adapt_hook_output(event_name, {"permission": "allow"})))
        else:
            print("{}")
        return 0

    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        print(json.dumps({"permission": "allow"}))
        return 0

    raw_name = str(event.get("hook_event_name", ""))
    event = normalize_tool_event(event)
    name = normalize_event_name(raw_name)

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

    out = adapt_hook_output(raw_name or name, out)
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
