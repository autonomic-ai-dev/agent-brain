/**
 * OpenCode plugin: require agent-brain route_task before other agent-brain MCP tools.
 * Loads from ~/.config/opencode/plugin/ or .opencode/plugin/
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

function routeGateScript(): string | null {
  const candidates = [
    join(homedir(), ".config", "opencode", "hooks", "agent-brain", "route_gate.py"),
    join(process.cwd(), ".opencode", "hooks", "agent-brain", "route_gate.py"),
  ];
  for (const path of candidates) {
    if (existsSync(path)) return path;
  }
  return null;
}

function runGate(event: Record<string, unknown>): Record<string, unknown> {
  const script = routeGateScript();
  if (!script) return { permission: "allow" };
  const res = spawnSync("python3", [script], {
    input: JSON.stringify(event),
    encoding: "utf-8",
    timeout: 25_000,
  });
  if (res.error) {
    return { permission: "allow" };
  }
  const out = (res.stdout || "").trim();
  if (!out) return {};
  try {
    return JSON.parse(out) as Record<string, unknown>;
  } catch {
    return {};
  }
}

function gateDenied(out: Record<string, unknown>): boolean {
  if (out.permission === "deny") return true;
  if (out.decision === "deny" || out.decision === "block") return true;
  const hs = out.hookSpecificOutput as Record<string, unknown> | undefined;
  return hs?.permissionDecision === "deny";
}

function denyMessage(out: Record<string, unknown>): string {
  return (
    (out.agent_message as string) ||
    (out.reason as string) ||
    (out.systemMessage as string) ||
    "Call agent-brain route_task first (user_message, cwd, open_files)."
  );
}

export const AgentBrainRouteGate = async () => {
  return {
    "tool.execute.before": async (
      input: { tool: string; sessionID?: string; callID?: string },
      output: { args: Record<string, unknown> },
    ) => {
      void output;
      const out = runGate({
        hook_event_name: "PreToolUse",
        tool_name: input.tool,
        tool_input: output.args,
      });
      if (gateDenied(out)) {
        throw new Error(denyMessage(out));
      }
    },
    "message": async (
      input: { role?: string },
      _output: unknown,
    ) => {
      if (input.role && input.role !== "user") return;
      runGate({ hook_event_name: "UserPromptSubmit" });
    },
  };
};

export default AgentBrainRouteGate;
