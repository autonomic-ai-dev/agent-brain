//! MCP-side turn gate — other agent-brain tools require a recent `route_task`.
//!
//! Cursor hooks enforce routing before *host* tools; this enforces routing before
//! *agent-brain MCP* tools on every host (OpenCode, Claude Code, VS Code, …).
//! Session digests and cross-agent memory only surface through `route_task`.

use std::sync::Mutex;
use std::time::Instant;

use rmcp::ErrorData as McpError;

use crate::cache::fingerprint_query;

#[derive(Debug, Default)]
struct GateState {
    last_route_at: Option<Instant>,
    user_message_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpRouteGate {
    inner: std::sync::Arc<Mutex<GateState>>,
    ttl_secs: u64,
    enabled: bool,
}

impl McpRouteGate {
    pub fn new(enabled: bool, ttl_secs: u64) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(GateState::default())),
            ttl_secs: ttl_secs.max(1),
            enabled,
        }
    }

    pub fn from_config(config: &crate::config::Config) -> Self {
        Self::new(config.mcp_gate_enabled, config.mcp_gate_ttl_secs)
    }

    pub fn record_route(&self, user_message: &str) {
        if !self.enabled {
            return;
        }
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state.last_route_at = Some(Instant::now());
        state.user_message_hash = Some(fingerprint_query(user_message));
    }

    pub fn require_route(&self, tool_name: &str) -> Result<(), McpError> {
        if !self.enabled {
            return Ok(());
        }
        let state = self
            .inner
            .lock()
            .map_err(|e| McpError::internal_error(format!("route gate lock: {e}"), None))?;
        let Some(at) = state.last_route_at else {
            return Err(gate_error(tool_name, GateReason::NotRouted));
        };
        if at.elapsed() > std::time::Duration::from_secs(self.ttl_secs) {
            return Err(gate_error(tool_name, GateReason::StaleRoute));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum GateReason {
    NotRouted,
    StaleRoute,
}

fn gate_error(tool_name: &str, reason: GateReason) -> McpError {
    let (code, detail) = match reason {
        GateReason::NotRouted => (
            "route_task_required",
            "No successful route_task in this MCP session yet.",
        ),
        GateReason::StaleRoute => (
            "route_task_stale",
            "The last route_task is too old for this turn.",
        ),
    };
    McpError::invalid_params(
        format!(
            "{detail} Call route_task with user_message before `{tool_name}`. \
             Skills, rules, session digests (Cursor/OpenCode/Codex/Gemini/Antigravity), and team memory \
             are injected only through route_task — bypassing it makes cross-agent ingest useless."
        ),
        Some(serde_json::json!({
            "error": code,
            "tool": tool_name,
            "required_first": "route_task",
            "hint": "agent-brain briefing shows the last successful route"
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_until_route_task() {
        let gate = McpRouteGate::new(true, 600);
        assert!(gate.require_route("grep_search").is_err());
        gate.record_route("fix the dashboard");
        assert!(gate.require_route("grep_search").is_ok());
    }

    #[test]
    fn disabled_gate_allows_tools() {
        let gate = McpRouteGate::new(false, 600);
        assert!(gate.require_route("store_memory").is_ok());
    }

    #[test]
    fn stale_route_blocks() {
        let gate = McpRouteGate::new(true, 1);
        gate.record_route("hello");
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(gate.require_route("get_context").is_err());
    }
}
