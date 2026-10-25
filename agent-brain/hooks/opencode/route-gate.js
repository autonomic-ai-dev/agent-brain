import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

function routeGateScript() {
  const candidates = [
    join(homedir(), ".config", "opencode", "hooks", "agent-brain", "route_gate.py"),
    join(process.cwd(), ".opencode", "hooks", "agent-brain", "route_gate.py"),
  ];
  for (const path of candidates) {
    if (existsSync(path)) return path;
  }
  return null;
}

function isRouteTask(tool) {
  const t = tool.toLowerCase();
  return (
    t === "route_task" ||
    t.endsWith("_route_task") ||
    t.endsWith(":route_task") ||
    (t.includes("route_task") && (t.includes("agent-brain") || t.includes("agent_brain")))
  );
}

function isAgentBrainTool(tool) {
  const t = tool.toLowerCase();
  if (isRouteTask(tool)) return true;
  return (
    t.startsWith("agent-brain_") ||
    t.startsWith("agent_brain_") ||
    t.startsWith("mcp_agent-brain_") ||
    t.startsWith("mcp_agent_brain_") ||
    t.startsWith("mcp__agent-brain__") ||
    t.startsWith("mcp__agent_brain__") ||
    (t.includes("agent-brain") && t.includes("mcp")) ||
    (t.includes("agent_brain") && t.includes("mcp"))
  );
}

function gateMissingPayload() {
  return {
    permission: "deny",
    agent_message: "agent-brain route gate script missing. Run: agent-brain install --opencode --global",
  };
}

function gateErrorPayload(message) {
  return { permission: "deny", agent_message: `agent-brain route gate error: ${message}` };
}

function runGate(event) {
  const tool = String(event.tool_name ?? "");
  const brainTool = isAgentBrainTool(tool);
  const script = routeGateScript();

  if (!script) {
    if (brainTool && !isRouteTask(tool)) return gateMissingPayload();
    return { permission: "allow" };
  }

  const res = spawnSync("python3", [script], {
    input: JSON.stringify({ server: "agent-brain", ...event }),
    encoding: "utf-8",
    timeout: 25_000,
  });

  if (res.error) {
    if (brainTool && !isRouteTask(tool)) return gateErrorPayload(res.error.message ?? "python3 failed");
    return { permission: "allow" };
  }

  const out = (res.stdout || "").trim();
  if (!out) return {};
  try {
    return JSON.parse(out);
  } catch {
    if (brainTool && !isRouteTask(tool)) return gateErrorPayload("invalid JSON from route_gate.py");
    return {};
  }
}

function gateDenied(out) {
  if (out.permission === "deny") return true;
  if (out.decision === "deny" || out.decision === "block") return true;
  return out.hookSpecificOutput?.permissionDecision === "deny";
}

function denyMessage(out) {
  return (
    out.agent_message ||
    out.reason ||
    out.systemMessage ||
    "Call agent-brain route_task first (user_message, cwd, open_files)."
  );
}

export default {
  id: "agent-brain-route-gate",
  async setup(ctx) {
    const before = await ctx.tool.hook("execute.before", (event) => {
      const out = runGate({
        hook_event_name: "PreToolUse",
        tool_name: event.tool,
        tool_input: event.input ?? {},
        session_id: event.sessionID,
        call_id: event.callID,
      });
      if (gateDenied(out)) throw new Error(denyMessage(out));
    });

    const after = await ctx.tool.hook("execute.after", (event) => {
      runGate({
        hook_event_name: "PostToolUse",
        tool_name: event.tool,
        tool_input: event.input ?? {},
        server: "agent-brain",
        success: event.status === "completed",
        session_id: event.sessionID,
        call_id: event.callID,
      });
    });

    const prompt = await ctx.session.hook("prompt", () => {
      runGate({ hook_event_name: "UserPromptSubmit" });
    });

    return async () => {
      await before.dispose();
      await after.dispose();
      await prompt.dispose();
    };
  },
};
