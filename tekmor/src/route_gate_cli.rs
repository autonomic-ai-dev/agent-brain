use std::io::Read;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use tekmor_core::gate_rule::{evaluate, GateInput, GateRule, GateVerdict};

use crate::route_gate_decision::{decide, Decision, GateState};

pub struct RouteRule {
    pub state: GateState,
    pub now: f64,
}

impl GateRule for RouteRule {
    fn check(&self, input: &GateInput<'_>) -> GateVerdict {
        match decide(input.tool, &self.state, self.now) {
            Decision::Allow | Decision::AllowWithoutRoute(_) => GateVerdict::Allow,
            Decision::Deny => GateVerdict::Block("route_task has not run this turn".into()),
        }
    }
}

const SESSION_KEYS: [&str; 12] = [
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
];

const SESSION_ENV: [&str; 4] = [
    "CURSOR_CONVERSATION_ID",
    "TEKMOR_HOOK_SESSION",
    "AGY_CONVERSATION_ID",
    "CODEX_THREAD_ID",
];

pub fn session_id(event: &Value) -> String {
    for key in SESSION_KEYS {
        if let Some(value) = event.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    for env in SESSION_ENV {
        if let Ok(value) = std::env::var(env) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "global".to_string()
}

fn tool_name(event: &Value) -> &str {
    ["tool_name", "tool", "name"]
        .iter()
        .find_map(|key| event.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty()))
        .unwrap_or("")
}

fn state_from_json(value: &Value) -> GateState {
    GateState {
        needs_route: value.get("needs_route").and_then(Value::as_bool).unwrap_or(false),
        needs_route_since: value.get("needs_route_since").and_then(Value::as_f64),
        route_grace_until: value.get("route_grace_until").and_then(Value::as_f64),
        mcp_offline_until: value.get("mcp_offline_until").and_then(Value::as_f64),
    }
}

pub fn read_session_state(state_path: &Path, session: &str) -> GateState {
    std::fs::read_to_string(state_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|root| root.get("sessions")?.get(session).map(state_from_json))
        .unwrap_or_default()
}

pub fn permission(event: &Value, state_path: &Path, now: f64) -> &'static str {
    let rule = RouteRule {
        state: read_session_state(state_path, &session_id(event)),
        now,
    };
    let input = GateInput {
        tool: tool_name(event),
        command: None,
    };
    match evaluate(&[&rule], &input) {
        GateVerdict::Block(_) => "deny",
        GateVerdict::Allow | GateVerdict::Warn(_) => "allow",
    }
}

pub fn run_cli(state_path: &Path) -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let event: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    let now = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
    println!("{}", json!({ "permission": permission(&event, state_path, now) }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000.0;

    fn state_file(sessions: Value) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), json!({ "version": 2, "sessions": sessions }).to_string()).unwrap();
        file
    }

    #[test]
    fn the_session_comes_from_the_first_known_event_key() {
        let event = json!({ "sessionId": " abc ", "conversation_id": "first" });
        assert_eq!(session_id(&event), "first");
        assert_eq!(session_id(&json!({ "sessionId": " abc " })), "abc");
    }

    #[test]
    fn a_pending_route_denies_a_read_in_that_session() {
        let file = state_file(json!({ "s1": { "needs_route": true, "needs_route_since": NOW } }));
        let event = json!({ "sessionId": "s1", "tool_name": "Read" });
        assert_eq!(permission(&event, file.path(), NOW), "deny");
    }

    #[test]
    fn another_session_pending_route_does_not_deny_this_one() {
        let file = state_file(json!({ "s1": { "needs_route": true, "needs_route_since": NOW } }));
        let event = json!({ "sessionId": "s2", "tool_name": "Read" });
        assert_eq!(permission(&event, file.path(), NOW), "allow");
    }

    #[test]
    fn route_task_is_allowed_even_while_a_route_is_pending() {
        let file = state_file(json!({ "s1": { "needs_route": true, "needs_route_since": NOW } }));
        let event = json!({ "sessionId": "s1", "tool_name": "mcp__tekmor__route_task" });
        assert_eq!(permission(&event, file.path(), NOW), "allow");
    }

    #[test]
    fn a_missing_state_file_allows() {
        let event = json!({ "sessionId": "s1", "tool_name": "Read" });
        assert_eq!(permission(&event, Path::new("/nonexistent/route_state.json"), NOW), "allow");
    }

    #[test]
    fn a_legacy_flat_state_file_is_not_read_as_a_session() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), json!({ "needs_route": true, "needs_route_since": NOW }).to_string())
            .unwrap();
        let event = json!({ "sessionId": "s1", "tool_name": "Read" });
        assert_eq!(permission(&event, file.path(), NOW), "allow");
    }
}
