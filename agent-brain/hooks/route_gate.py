#!/usr/bin/env python3
"""Cursor hook: require agent-brain route_task before other tools each user turn."""

from __future__ import annotations

import re
import shutil
import subprocess

import json
import os
import sys
import time
from pathlib import Path

def default_brain_home() -> Path:
    explicit = os.environ.get("AGENT_BRAIN_HOME", "").strip()
    if explicit:
        return Path(explicit).expanduser()
    autonomic = Path.home() / ".autonomic" / "memory"
    legacy = Path.home() / ".agent_brain"
    if (autonomic / "brain.db").exists() or not legacy.exists():
        return autonomic
    return legacy


STATE_PATH = default_brain_home() / "hooks" / "route_state.json"
# Thread-local / call-scoped session for load_state/save_state without threading event everywhere.
_ACTIVE_SESSION: str | None = None

ROUTE_TOOL_NAMES = {
    "route_task",
    "MCP:route_task",
    "mcp:route_task",
    "mcp_agent-brain_route_task",
    "mcp__agent-brain__route_task",
    "agent-brain_route_task",
    "agent_brain_route_task",
}

# Meta-discovery tools used across clients (e.g. Claude Code ToolSearch, Cursor/Codex mcp_list_tools)
# to search, inspect, or load tool schemas before invoking route_task. Must never be blocked by the gate.
META_DISCOVERY_TOOL_NAMES = {
    "toolsearch",
    "tool_search",
    "tool-search",
    "list_tools",
    "listtools",
    "list-tools",
    "search_tools",
    "searchtools",
    "search-tools",
    "mcp_list_tools",
    "mcp_search_tools",
}


def is_meta_or_discovery_tool(event: dict) -> bool:
    tool = str(event.get("tool_name") or event.get("tool") or event.get("name") or "").strip().lower()
    if not tool:
        return False
    if tool in META_DISCOVERY_TOOL_NAMES:
        return True
    clean = tool
    for prefix in ("mcp__", "mcp_", "mcp:"):
        if clean.startswith(prefix):
            clean = clean[len(prefix):]
    if clean in META_DISCOVERY_TOOL_NAMES:
        return True
    return False

# OpenCode exposes MCP tools as agent-brain_<tool> (no mcp_ prefix).
OPENCODE_BRAIN_PREFIXES = ("agent-brain_", "agent_brain_")

GRACE_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_GRACE_SECS", "120"))
STALE_ROUTE_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_STALE_SECS", "45"))
OFFLINE_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_OFFLINE_SECS", "1800"))
SESSION_ROUTE_TTL_SECS = float(os.environ.get("AGENT_BRAIN_ROUTE_SESSION_TTL_SECS", "14400"))
# all = gate every tool (hard blocking across all agents); brain_mcp = gate only agent-brain tools.
GATE_SCOPE = os.environ.get("AGENT_BRAIN_ROUTE_GATE_SCOPE", "all").strip().lower()
READ_GATE_MODE = os.environ.get("AGENT_BRAIN_READ_GATE", "hard").strip().lower()
READ_WARN_BYTES = int(os.environ.get("AGENT_BRAIN_READ_WARN_BYTES", "65536"))
# After a bounded MCP scout on a path, allow native Read/StrReplace for this long
# (avoids Shell+python workarounds on large .rs/.md files).
SCOUT_UNLOCK_SECS = float(os.environ.get("AGENT_BRAIN_SCOUT_UNLOCK_SECS", "3600"))
DEFAULT_BOUNDED_READ_LINES = int(os.environ.get("AGENT_BRAIN_BOUNDED_READ_LINES", "400"))
SCOUT_TOOL_MARKERS = (
    "file_summary",
    "read_file_head",
    "read_file_tail",
    "read_file",
    "list_directory",
    "find_files",
    "grep_search",
)
# Tool-level enforcement for coding-policy §3 (instructions alone do not stop Shell+python).
ALLOW_PYTHON_SHELL = os.environ.get("AGENT_BRAIN_ALLOW_PYTHON_SHELL", "").strip().lower() in {
    "1",
    "true",
    "yes",
    "on",
}
# Command-position only (not prose like commit messages mentioning "python -c").
# (?<![.\w]) avoids matching the "py" suffix of "*.py" paths in the same Shell line.
_PY = r"(?<![.\w])(?:python3?|py)(?:\d+(?:\.\d+)*)?\b"
# No backtick in the prefix class: markdown/`python -c` in prose must not match.
# $(python …) still matches via the '(' alternative.
_PY_CMD = rf"(?:^|[;&|$\n(]\s*|(?:sudo\s+)|(?:env\s+[^\n]*\s)){_PY}"
PYTHON_INVOKE_RE = re.compile(rf"(?i){_PY_CMD}")
# Deny agent workarounds: -c, heredoc, stdin pipe/mode. Allow python -m pytest / foo.py runs.
PYTHON_HEREDOC_RE = re.compile(rf"(?i){_PY_CMD}[^\n]*<<")
PYTHON_DASH_C_RE = re.compile(rf"(?i){_PY_CMD}(?:\s+-\w+)*\s+-c\b")
PYTHON_STDIN_PIPE_RE = re.compile(rf"(?i)\|\s*(?:sudo\s+)?{_PY}")
PYTHON_STDIN_MODE_RE = re.compile(rf"(?i){_PY_CMD}(?:\s+-\w+)*\s+-(?:\s|$|[;&|])")
PYTHON_DENY_AGENT = (
    "agent-brain: Python via Shell denied (coding-policy §3 tool policy). "
    "Do not use python -c / heredoc / piped stdin to read or edit files. "
    "Use MCP file_summary/read_file_head/grep_search then Read/StrReplace, or rg/sed/awk. "
    "Allowed: python -m pytest|unittest|… and running project .py files. "
    "Override only with user approval: AGENT_BRAIN_ALLOW_PYTHON_SHELL=1."
)
BLOCKED_PATH_SEGMENTS = ("dist", "node_modules", "target", "build", ".git", ".next", "coverage")
TOOL_EVENTS_PATH = STATE_PATH.parent / "tool_events.jsonl"


def disabled() -> bool:
    v = os.environ.get("AGENT_BRAIN_ROUTE_HOOKS", "1").strip().lower()
    return v in {"0", "false", "no", "off"}


def session_id_from_event(event: dict | None) -> str:
    """Isolate route gate state per conversation/host so concurrent agents do not clobber each other."""
    event = event or {}
    for key in (
        "conversation_id",
        "conversationId",
        "session_id",
        "sessionId",
        "composerId",
        "composer_id",
        "agent_id",
        "agentId",
        "generation_id",
        "generationId",
        "turn_id",
        "turnId",
    ):
        val = event.get(key)
        if isinstance(val, str) and val.strip():
            return val.strip()
    for env in (
        "CURSOR_CONVERSATION_ID",
        "AGENT_BRAIN_HOOK_SESSION",
        "AGY_CONVERSATION_ID",
        "CODEX_THREAD_ID",
    ):
        val = os.environ.get(env, "").strip()
        if val:
            return val
    return "global"


def set_active_session(session_id: str | None) -> None:
    global _ACTIVE_SESSION
    _ACTIVE_SESSION = session_id


def active_session() -> str:
    return _ACTIVE_SESSION or "global"


def _load_root() -> dict:
    if not STATE_PATH.exists():
        return {"version": 2, "sessions": {}}
    try:
        raw = json.loads(STATE_PATH.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return {"version": 2, "sessions": {}}
    if isinstance(raw, dict) and isinstance(raw.get("sessions"), dict):
        raw.setdefault("version", 2)
        return raw
    # Legacy flat state — claim into the first session that loads it.
    if isinstance(raw, dict):
        return {"version": 2, "sessions": {}, "_orphan_legacy": raw}
    return {"version": 2, "sessions": {}}


def _prune_sessions(root: dict, keep: str, max_age_secs: float = 86400.0) -> None:
    sessions = root.get("sessions")
    if not isinstance(sessions, dict):
        return
    now = time.time()
    drop = []
    for sid, st in sessions.items():
        if sid == keep:
            continue
        if not isinstance(st, dict):
            drop.append(sid)
            continue
        stamp = st.get("route_context_at") or st.get("needs_route_since") or 0
        if isinstance(stamp, (int, float)) and stamp > 0 and (now - stamp) > max_age_secs:
            drop.append(sid)
    for sid in drop:
        sessions.pop(sid, None)


def load_state(event: dict | None = None) -> dict:
    if event is not None:
        set_active_session(session_id_from_event(event))
    sid = active_session()
    root = _load_root()
    sessions = root.setdefault("sessions", {})
    if sid not in sessions:
        if isinstance(root.get("_orphan_legacy"), dict):
            sessions[sid] = root.pop("_orphan_legacy")
            # Persist migration so concurrent hooks see the same owner.
            try:
                STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
                STATE_PATH.write_text(json.dumps(root), encoding="utf-8")
            except OSError:
                pass
        else:
            sessions[sid] = {}
    st = sessions.get(sid)
    return dict(st) if isinstance(st, dict) else {}


def save_state(state: dict, event: dict | None = None) -> None:
    if event is not None:
        set_active_session(session_id_from_event(event))
    sid = active_session()
    root = _load_root()
    sessions = root.setdefault("sessions", {})
    sessions[sid] = state
    _prune_sessions(root, sid)
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    STATE_PATH.write_text(json.dumps(root), encoding="utf-8")


def claude_mcp_parts(tool_name: str) -> tuple[str | None, str | None]:
    """Parse Claude Code MCP tool names: mcp__<server>__<tool>."""
    tool = tool_name.strip()
    if not tool.lower().startswith("mcp__"):
        return None, None
    parts = tool.split("__", 2)
    if len(parts) < 3 or not parts[1] or not parts[2]:
        return None, None
    return parts[1], parts[2]


def is_claude_agent_brain_mcp(tool_name: str) -> bool:
    server, _tool = claude_mcp_parts(tool_name)
    if server is None:
        return False
    normalized = server.lower().replace("_", "-")
    return normalized in {"agent-brain", "agentbrain", "agent-body", "agentbody"}


def is_opencode_brain_tool(tool: str) -> bool:
    tool_lower = tool.lower()
    return any(tool_lower.startswith(prefix) for prefix in OPENCODE_BRAIN_PREFIXES)


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
    _server, claude_tool = claude_mcp_parts(tool)
    if claude_tool and claude_tool.lower() == "route_task" and is_claude_agent_brain_mcp(tool):
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
    if is_opencode_brain_tool(tool):
        return True
    if is_claude_agent_brain_mcp(tool):
        return True
    if tool_lower.startswith("mcp_agent-brain_") or tool_lower.startswith("mcp_agent-body_"):
        return True
    if tool_lower.startswith("mcp_agent_brain_") or tool_lower.startswith("mcp_agent_body_"):
        return True
    if tool_lower.startswith("mcp_agent-brain-") or tool_lower.startswith("mcp_agent-body-"):
        return True
    if tool_lower.startswith("mcp_agent_brain-") or tool_lower.startswith("mcp_agent_body-"):
        return True
    if ("agent-brain" in tool_lower or "agent-body" in tool_lower) and "mcp" in tool_lower:
        return True
    server = str(event.get("server") or "").strip().lower()
    if server in {"agent-brain", "agent_brain", "agent-body", "agent_body"}:
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
    if isinstance(parsed, list):
        parsed = {"content": parsed}
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


def write_edit_memory_suggestion(path: str) -> None:
    state = load_state()
    name = Path(path).name
    state["edit_memory_suggestion"] = {
        "topic": f"session-edit-{name}",
        "fact": f"Edited {path} this session — capture conventions if durable.",
        "path": path,
        "suggested_at": time.time(),
        "apply_with": "store_memory",
    }
    save_state(state)


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
    if is_cursor_edit_tool(event):
        return False
    return tool in {
        "read",
        "mcp_read",
        "read_file",
        "readfile",
        "readfiletool",
        "fileread",
        "view",
        "view_file",
    } or tool.endswith("_read") or tool.startswith("read_") or tool.startswith("view_")


def is_cursor_edit_tool(event: dict) -> bool:
    """Write/StrReplace/ApplyPatch — never subject to the large-file Read gate."""
    tool = str(event.get("tool_name") or "").strip().lower()
    return any(
        marker in tool
        for marker in (
            "write",
            "strreplace",
            "search_replace",
            "apply_patch",
            "applypatch",
            "editnotebook",
            "edit_notebook",
            "fileedit",
            "replace_file_content",
            "write_to_file",
        )
    ) or tool in {"edit", "searchreplace"}


def extract_read_path(event: dict) -> str | None:
    for key in ("tool_input", "arguments", "args", "input"):
        raw = event.get(key)
        parsed = parse_json_value(raw)
        if isinstance(parsed, dict):
            for field in (
                "path",
                "target_file",
                "file",
                "file_path",
                "filePath",
                "AbsolutePath",
                "absolutePath",
                "TargetFile",
                "targetFile",
            ):
                value = parsed.get(field)
                if isinstance(value, str) and value.strip():
                    return value.strip()
    return None


def normalize_scout_path(path: str) -> str:
    try:
        return str(Path(path).expanduser().resolve())
    except OSError:
        return path.strip()


def record_scouted_path(path: str, event: dict | None = None) -> None:
    if not path or not path.strip():
        return
    state = load_state(event)
    scouted = state.get("scouted_paths")
    if not isinstance(scouted, dict):
        scouted = {}
    key = normalize_scout_path(path)
    scouted[key] = time.time()
    # Keep map bounded.
    if len(scouted) > 64:
        oldest = sorted(scouted.items(), key=lambda kv: kv[1])[: len(scouted) - 64]
        for dead, _ in oldest:
            scouted.pop(dead, None)
    state["scouted_paths"] = scouted
    save_state(state, event)


def path_is_scouted(state: dict, path: str) -> bool:
    scouted = state.get("scouted_paths")
    if not isinstance(scouted, dict) or not path:
        return False
    key = normalize_scout_path(path)
    now = time.time()
    for stored, ts in scouted.items():
        if not isinstance(ts, (int, float)):
            continue
        if now - ts > SCOUT_UNLOCK_SECS:
            continue
        if stored == key or stored.endswith("/" + Path(key).name) or key.endswith(stored):
            return True
        try:
            if Path(stored).resolve() == Path(key).resolve():
                return True
        except OSError:
            pass
    return False


def is_bounded_native_read(event: dict) -> bool:
    """Allow native Read when the host already requested a line slice."""
    for key in ("tool_input", "arguments", "args", "input"):
        raw = event.get(key)
        parsed = parse_json_value(raw)
        if not isinstance(parsed, dict):
            continue
        limit = parsed.get("limit")
        if isinstance(limit, int) and 0 < limit <= DEFAULT_BOUNDED_READ_LINES:
            return True
        # Cursor sometimes uses end_line/start_line or offset.
        if parsed.get("offset") is not None and limit is not None:
            return True
        end_line = parsed.get("end_line") or parsed.get("endLine")
        start_line = parsed.get("start_line") or parsed.get("startLine") or 1
        if isinstance(end_line, int) and isinstance(start_line, int):
            if 0 < (end_line - start_line + 1) <= DEFAULT_BOUNDED_READ_LINES:
                return True
    return False


def maybe_record_scout_from_event(event: dict) -> None:
    """Unlock a path after successful MCP file_summary / read_file_* / grep_search."""
    if event.get("success") is False or event.get("error"):
        return
    tool = str(event.get("tool_name") or "").lower()
    if not any(marker in tool for marker in SCOUT_TOOL_MARKERS):
        return
    if not (
        is_agent_brain_mcp_tool(event)
        or "agent-brain" in tool
        or "agent_brain" in tool
        or tool in SCOUT_TOOL_MARKERS
    ):
        return
    path = extract_read_path(event)
    if path:
        record_scouted_path(path, event)


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
    # Edits must never be blocked by the large-file Read policy (forces Shell+python otherwise).
    if is_cursor_edit_tool(event):
        return None
    if not is_cursor_read_tool(event):
        return None

    state = load_state(event)
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

    # Bounded native Read (limit/offset) or prior MCP scout → allow (enables StrReplace on large files).
    if path and (is_bounded_native_read(event) or path_is_scouted(state, path)):
        return None

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
        if READ_GATE_MODE == "hard":
            detail = (
                "must_apply constraints are active"
                if must_apply
                else f"file exceeds {READ_WARN_BYTES} bytes"
            )
            return {
                "permission": "deny",
                "agent_message": (
                    f"Read denied ({detail}). First call agent-brain {tool_hint} on this path "
                    f"(unlocks native Read/StrReplace for {int(SCOUT_UNLOCK_SECS)}s). "
                    "Do not patch via Shell+python."
                ),
                "user_message": (
                    "agent-brain: scout with file_summary/read_file_head first, then StrReplace — "
                    "no Shell+python."
                ),
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
    state = load_state(event)
    state["needs_route"] = False
    state.pop("needs_route_since", None)
    state["route_grace_until"] = 0
    state["routed_at"] = time.time()
    state["routed_cwd"] = current_cwd(event)
    clear_mcp_offline(state)
    gen = event.get("generation_id") or event.get("generationId") or state.get("pending_generation")
    if isinstance(gen, str) and gen.strip():
        state["generation_id"] = gen.strip()
        state["routed_generation"] = gen.strip()
    save_state(state, event)


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


def current_cwd(event: dict | None) -> str:
    cwd = (event or {}).get("cwd")
    return str(cwd) if cwd else os.getcwd()


def session_route_reusable(state: dict, cwd: str) -> bool:
    routed_at = state.get("routed_at")
    if state.get("needs_route") or not isinstance(routed_at, (int, float)):
        return False
    if state.get("routed_cwd") != cwd:
        return False
    return time.time() - routed_at < SESSION_ROUTE_TTL_SECS


def handle_before_submit_prompt(event: dict) -> dict:
    state = load_state(event)
    if in_mcp_offline(state):
        save_state(state, event)
        return {"continue": True}
    gen = None
    for key in ("generation_id", "generationId"):
        val = event.get(key)
        if isinstance(val, str) and val.strip():
            gen = val.strip()
            break
    # Mid-turn / duplicate UserPromptSubmit for the same generation must NOT re-arm
    # the gate after a successful route_task (this was the main mid-turn flake).
    if gen and state.get("routed_generation") == gen and not state.get("needs_route"):
        return {"continue": True}
    if session_route_reusable(state, current_cwd(event)):
        state["last_turn_ping"] = time.time()
        save_state(state, event)
        return {"continue": True}
    state["needs_route"] = True
    state["needs_route_since"] = time.time()
    state["route_grace_until"] = 0
    if gen:
        state["pending_generation"] = gen
    save_state(state, event)
    return {"continue": True}


def handle_after_mcp_execution(event: dict) -> dict:
    handle_route_outcome(event)
    maybe_record_scout_from_event(event)
    return {}


def handle_post_tool_use(event: dict) -> dict:
    # Cursor Agent MCP tools (mcp_agent-brain_*) clear the gate via postToolUse,
    # not afterMCPExecution.
    handle_route_outcome(event)
    maybe_log_tool_trace(event)
    maybe_record_scout_from_event(event)
    maybe_record_verify_outcome(event)
    return {}


def extract_shell_command(event: dict, *, max_len: int | None = 240) -> str | None:
    """Return the Shell command string. Truncate for traces; pass max_len=None for policy checks."""
    candidates: list[str] = []
    top = event.get("command")
    if isinstance(top, str) and top.strip():
        candidates.append(top.strip())
    for key in ("tool_input", "arguments", "args", "input"):
        raw = event.get(key)
        parsed = parse_json_value(raw)
        if isinstance(parsed, dict):
            for field in ("command", "cmd", "CommandLine", "commandLine", "script", "code"):
                value = parsed.get(field)
                if isinstance(value, str) and value.strip():
                    candidates.append(value.strip())
        elif isinstance(parsed, str) and parsed.strip():
            candidates.append(parsed.strip())
    if not candidates:
        return None
    cmd = max(candidates, key=len)
    if max_len is not None and len(cmd) > max_len:
        return cmd[:max_len]
    return cmd


def python_shell_deny_payload(reason: str) -> dict:
    msg = f"{PYTHON_DENY_AGENT} Denied pattern: {reason}."
    return {
        "permission": "deny",
        "agent_message": msg,
        "user_message": f"agent-brain: blocked Python Shell ({reason}).",
    }


def check_python_shell_policy(cmd: str | None) -> dict | None:
    """Deny Shell python -c / heredoc / stdin workarounds; allow project runs and -m tools."""
    if ALLOW_PYTHON_SHELL or not cmd:
        return None
    if not PYTHON_INVOKE_RE.search(cmd) and not PYTHON_STDIN_PIPE_RE.search(cmd):
        return None
    if PYTHON_HEREDOC_RE.search(cmd):
        return python_shell_deny_payload("heredoc")
    if PYTHON_DASH_C_RE.search(cmd):
        return python_shell_deny_payload("python -c")
    if PYTHON_STDIN_PIPE_RE.search(cmd):
        return python_shell_deny_payload("piped to python")
    if PYTHON_STDIN_MODE_RE.search(cmd):
        return python_shell_deny_payload("python stdin (-)")
    return None


SHELL_INVOKERS = {"bash", "sh", "zsh", "dash", "ksh", "source", "."}
SCRIPT_SUFFIXES = (".sh", ".bash", ".zsh")
SCRIPT_SIZE_LIMIT = 256 * 1024


def referenced_script_paths(cmd: str) -> list[str]:
    tokens = cmd.split()
    paths = []
    for index, token in enumerate(tokens):
        if token in SHELL_INVOKERS and index + 1 < len(tokens):
            candidate = tokens[index + 1]
        elif token.startswith(("./", "/")) and token.endswith(SCRIPT_SUFFIXES):
            candidate = token
        else:
            continue
        if os.path.isfile(candidate) and os.path.getsize(candidate) <= SCRIPT_SIZE_LIMIT:
            paths.append(candidate)
    return list(dict.fromkeys(paths))


def check_mouth_ast_policy(cmd: str | None) -> dict | None:
    """Validate bash command and any script it runs against agent-mouth AST approval policy if available."""
    if not cmd:
        return None
    stripped = cmd.strip()
    if not stripped:
        return None
    mouth_bin = shutil.which("agent-mouth") or "/opt/homebrew/bin/agent-mouth"
    if not os.path.exists(mouth_bin):
        return None
    targets = [["--command", stripped]]
    targets += [["--script", path] for path in referenced_script_paths(stripped)]
    for target in targets:
        try:
            res = subprocess.run(
                [mouth_bin, "validate", *target],
                capture_output=True,
                text=True,
                timeout=2.0,
            )
        except (subprocess.SubprocessError, OSError) as exc:
            return mouth_denial(f"validation unavailable ({type(exc).__name__}); failing closed")
        if res.returncode != 0:
            reason = "destructive command pattern detected"
            try:
                data = json.loads(res.stdout)
                reason = data.get("reason", reason)
            except (ValueError, AttributeError):
                pass
            return mouth_denial(reason)
    return None


def mouth_denial(reason: str) -> dict:
    return {
        "permission": "deny",
        "agent_message": (
            f"agent-mouth AST safety violation: {reason}. Destructive host execution "
            "is blocked. Delegate to agent-muscle (muscle_execute_bash) or request explicit user approval."
        ),
        "user_message": f"agent-mouth: blocked command ({reason}).",
    }


VERIFICATION_CMD_RE = re.compile(
    r"(?i)(?:^|[;&|\n]\s*)(?:"
    r"cargo\s+(?:test|clippy|check)\b|"
    r"npm\s+test\b|"
    r"npx\s+(?:vitest|jest|playwright)\b|"
    r"bun\s+test\b|"
    r"pytest\b|"
    r"go\s+test\b|"
    r"make\s+test\b|"
    r"agent-brain\s+(?:proofs|doctor)\b|"
    r"\.verify/"
    r")"
)


def is_shell_tool(tool: str) -> bool:
    """True for Shell/Bash/terminal tools across Cursor, Claude, Codex, Gemini, OpenCode."""
    t = tool.lower().strip()
    if not t:
        return False
    if t in {
        "bash",
        "shell",
        "zsh",
        "sh",
        "terminal",
        "run_terminal_cmd",
        "run_terminal",
        "run_command",
        "shell_command",
        "execute",
        "exec",
        "powershell",
        "pwsh",
        "cmd",
    }:
        return True
    return "shell" in t or "terminal" in t or t.endswith("_bash") or t.endswith(".bash")


def looks_like_verification(cmd: str) -> bool:
    if not cmd or "agent-brain verify" in cmd:
        return False
    if ".verify/" in cmd:
        return True
    return VERIFICATION_CMD_RE.search(cmd) is not None


def shell_exit_code(event: dict) -> int:
    if event.get("success") is False or event.get("error"):
        return 1
    return 0


def agent_brain_bin() -> str:
    home = Path.home()
    local = home / ".local" / "bin" / "agent-brain"
    if local.is_file():
        return str(local)
    which = shutil.which("agent-brain")
    return which or "agent-brain"


def maybe_record_verify_outcome(event: dict) -> None:
    """Fire-and-forget: record matching Shell verification commands."""
    tool = str(event.get("tool_name") or "").lower()
    if not is_shell_tool(tool):
        return
    cmd = extract_shell_command(event)
    if not cmd or not looks_like_verification(cmd):
        return
    # Prefer full command if available (extract truncates to 240 for traces).
    full = cmd
    for key in ("tool_input", "arguments", "args", "input"):
        raw = event.get(key)
        parsed = parse_json_value(raw)
        if isinstance(parsed, dict):
            for field in ("command", "cmd"):
                value = parsed.get(field)
                if isinstance(value, str) and value.strip():
                    full = value.strip()
                    break
        elif isinstance(parsed, str) and parsed.strip():
            full = parsed.strip()
        if full:
            break
    exit_code = shell_exit_code(event)
    cwd = None
    for key in ("tool_input", "arguments", "args", "input"):
        raw = event.get(key)
        parsed = parse_json_value(raw)
        if isinstance(parsed, dict):
            for field in ("working_directory", "cwd", "workingDirectory"):
                value = parsed.get(field)
                if isinstance(value, str) and value.strip():
                    cwd = value.strip()
                    break
        if cwd:
            break
    bin_path = agent_brain_bin()
    argv = [
        bin_path,
        "verify",
        "record",
        "--exit",
        str(exit_code),
        "--source",
        "hook",
        "--",
        full,
    ]
    try:
        kwargs = {
            "stdout": subprocess.DEVNULL,
            "stderr": subprocess.DEVNULL,
            "start_new_session": True,
        }
        if cwd:
            kwargs["cwd"] = cwd
        subprocess.Popen(argv, **kwargs)
    except Exception:
        pass


def maybe_log_tool_trace(event: dict) -> None:
    if event.get("success") is False or event.get("error"):
        return
    tool = str(event.get("tool_name") or "").lower()
    detail = None
    path = None
    if "shell" in tool or "terminal" in tool or tool in {"run_terminal_cmd", "bash"}:
        detail = extract_shell_command(event)
    elif any(x in tool for x in ("write", "strreplace", "search_replace", "edit")):
        path = extract_read_path(event)
        if path:
            detail = f"edited {path}"
            write_edit_memory_suggestion(path)
    if not detail and not path:
        return
    state = load_state()
    append_tool_event(
        {
            "timestamp": int(time.time() * 1000),
            "tool_name": tool,
            "path": path,
            "detail": detail,
            "tokens_used": 0,
            "must_apply_active": bool(state.get("must_apply")),
            "phase": state.get("route_phase"),
        }
    )


def gate_tool_use(event: dict) -> dict:
    if is_route_task(event):
        return {"permission": "allow"}
    if is_meta_or_discovery_tool(event):
        return {"permission": "allow"}
    if not should_gate_tool(event):
        return {"permission": "allow"}
    state = load_state(event)
    if state.get("needs_route"):
        if should_allow_without_route(state):
            return grace_allow_payload(state)
        return deny_payload()
    return {"permission": "allow"}


def handle_pre_tool_use(event: dict) -> dict:
    read_gate = check_read_tool_gate(event)
    if read_gate is not None:
        return read_gate
    tool = str(event.get("tool_name") or "")
    if is_shell_tool(tool):
        cmd = extract_shell_command(event, max_len=None)
        denied = check_python_shell_policy(cmd)
        if denied is not None:
            return denied
        mouth_denied = check_mouth_ast_policy(cmd)
        if mouth_denied is not None:
            return mouth_denied
    return gate_tool_use(event)


def handle_before_mcp_execution(event: dict) -> dict:
    return gate_tool_use(event)


def handle_before_shell_execution(event: dict) -> dict:
    """Host shell pre-hook (Cursor beforeShellExecution) — gate route, Python deny, mouth AST validation, failClosed."""
    gate = gate_tool_use(event)
    if gate.get("permission") == "deny":
        return gate
    cmd = extract_shell_command(event, max_len=None)
    denied = check_python_shell_policy(cmd)
    if denied is not None:
        return denied
    mouth_denied = check_mouth_ast_policy(cmd)
    if mouth_denied is not None:
        return mouth_denied
    return {"permission": "allow"}


def is_codex_event(event: dict | None) -> bool:
    """Codex turn hooks include turn_id; bare permissionDecision allow is unsupported."""
    if not event:
        return False
    if event.get("turn_id"):
        return True
    return bool(os.environ.get("CODEX_HOME"))


def adapt_pre_tool_use_output(out: dict, event: dict | None) -> dict:
    """Map route gate payloads to Claude Code / Codex PreToolUse schemas."""
    permission = out.get("permission")
    agent_message = out.get("agent_message") or out.get("user_message") or ""
    if permission == "deny":
        return {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": agent_message,
            }
        }
    if permission == "allow":
        if is_codex_event(event):
            # Codex marks bare permissionDecision allow as invalid unless updatedInput is set.
            if agent_message:
                return {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "additionalContext": agent_message,
                    }
                }
            return {}
        if agent_message:
            return {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                },
                "systemMessage": agent_message,
            }
        return {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
            }
        }
    return out


def adapt_hook_output(event_name: str, out: dict, event: dict | None = None) -> dict:
    """Map Cursor-style permission payloads to Claude Code / Gemini / Codex hook schemas."""
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
    # Cursor-native events use permission/user_message/agent_message directly.
    if event_name in {"preToolUse", "beforeMCPExecution", "beforeShellExecution"}:
        return out
    if event_name == "PreToolUse":
        return adapt_pre_tool_use_output(out, event)
    if event_name == "UserPromptSubmit":
        if out.get("continue") is False:
            return {"decision": "block", "reason": agent_message}
        return {}
    if event_name == "BeforeAgent":
        if out.get("continue") is False:
            return {"decision": "deny", "reason": agent_message}
        return {"decision": "allow"}
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
            print("{}")
        elif event_name in {
            "preToolUse",
            "PreToolUse",
            "BeforeTool",
            "beforeMCPExecution",
            "beforeShellExecution",
        }:
            print(json.dumps(adapt_hook_output(event_name, {"permission": "allow"}, event)))
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
    set_active_session(session_id_from_event(event))
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
    elif name == "beforeShellExecution":
        out = handle_before_shell_execution(event)
    else:
        out = {}

    out = adapt_hook_output(raw_name or name, out, event)
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
