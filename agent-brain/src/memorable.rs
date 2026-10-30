use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::db::store::BrainStore;
use crate::settings::{AgentBrainSettings, UpstreamServerConfig};
use crate::verify::last_verification;

pub const SERVER_NAME: &str = "memorable";

const COMMAND: &str = "memorable";
const UNREACHABLE_API_URL: &str = "http://127.0.0.1:9";
const TARGET_KEYS: [&str; 5] = ["file_path", "path", "command", "pattern", "url"];
const SECRET_MARKERS: [&str; 6] = ["sk-", "ghp_", "AKIA", "xox", "-----BEGIN", "Bearer "];
const MAX_TARGET_CHARS: usize = 200;

pub fn is_registered(settings: &AgentBrainSettings) -> bool {
    settings
        .upstream_mcp
        .servers
        .iter()
        .any(|server| server.name == SERVER_NAME)
}

pub fn enable(settings: &mut AgentBrainSettings, capture: bool) {
    settings.memorable.enabled = true;
    settings.memorable.capture = capture;
    settings
        .upstream_mcp
        .servers
        .retain(|server| server.name != SERVER_NAME);
    settings.upstream_mcp.servers.push(UpstreamServerConfig {
        name: SERVER_NAME.to_string(),
        command: COMMAND.to_string(),
        args: vec!["mcp".to_string()],
        env: HashMap::from([(
            "MEMORABLE_API_URL".to_string(),
            UNREACHABLE_API_URL.to_string(),
        )]),
        enabled: true,
    });
}

pub fn disable(settings: &mut AgentBrainSettings) {
    settings.memorable.enabled = false;
    settings.memorable.capture = false;
    settings
        .upstream_mcp
        .servers
        .retain(|server| server.name != SERVER_NAME);
}

pub fn capture_allowed(settings: &AgentBrainSettings, last_passing: bool) -> Result<()> {
    if !settings.memorable.enabled || !settings.memorable.capture {
        bail!("memorable capture is off; run `agent-brain memorable enable --capture` to allow it");
    }
    if !last_passing {
        bail!("the last `agent-brain verify` for this repo did not pass; capture only verified sessions");
    }
    Ok(())
}

pub fn reduce_trace(trace: &Value) -> Result<Value> {
    let calls = trace
        .get("tool_calls")
        .and_then(Value::as_array)
        .context("trace needs a tool_calls array")?;
    let reduced: Vec<Value> = calls.iter().map(reduce_call).collect();
    Ok(json!({
        "session_id": trace.get("session_id").and_then(Value::as_str).unwrap_or("capture"),
        "tool_calls": reduced,
    }))
}

fn reduce_call(call: &Value) -> Value {
    let name = call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let target = TARGET_KEYS
        .iter()
        .find_map(|key| call.get("input")?.get(*key)?.as_str())
        .map(|value| value.chars().take(MAX_TARGET_CHARS).collect::<String>());
    let input = match target {
        Some(value) => json!({ "target": value }),
        None => json!({}),
    };
    let result = match call
        .get("result")
        .and_then(|r| r.get("exit_code"))
        .and_then(Value::as_i64)
    {
        Some(code) => json!({ "exit_code": code }),
        None => json!({}),
    };
    json!({ "name": name, "input": input, "result": result })
}

pub fn contains_secret(text: &str) -> bool {
    SECRET_MARKERS.iter().any(|marker| text.contains(marker))
}

pub fn capture(home: &Path, db_path: &Path, cwd: &Path, trace_path: &Path) -> Result<String> {
    let project = crate::project_key::project_key_if_known(cwd)
        .context("capture needs a repo with an agent-brain project key")?;
    let store = BrainStore::open(db_path)?;
    let last_passing = last_verification(&store, &project).is_some_and(|status| status.passing);
    let settings = AgentBrainSettings::load(home);
    capture_allowed(&settings, last_passing)?;

    let raw = std::fs::read_to_string(trace_path)
        .with_context(|| format!("read {}", trace_path.display()))?;
    let trace: Value = serde_json::from_str(&raw).context("trace is not valid JSON")?;
    let reduced = reduce_trace(&trace)?;
    let serialized = serde_json::to_string(&reduced)?;
    if contains_secret(&serialized) {
        bail!("trace contains a credential-shaped value; nothing was sent");
    }

    let outbox = outbox_path(home);
    std::fs::create_dir_all(outbox.parent().context("outbox has a parent")?)?;
    std::fs::write(&outbox, format!("{serialized}\n"))
        .with_context(|| format!("write {}", outbox.display()))?;
    let output = Command::new(COMMAND)
        .arg("ingest")
        .arg(&outbox)
        .output()
        .context("run memorable ingest (is the memorable binary on PATH?)")?;
    let _ = std::fs::remove_file(&outbox);
    if !output.status.success() {
        bail!(
            "memorable ingest failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn outbox_path(home: &Path) -> PathBuf {
    home.join("memorable").join("outbox.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace() -> Value {
        json!({
            "session_id": "s1",
            "title": "private task title",
            "tool_calls": [
                {"name": "Edit", "input": {"file_path": "src/lib.rs", "old_string": "secret body"}, "result": {"ok": true, "stdout": "file contents"}},
                {"name": "Bash", "input": {"command": "cargo test"}, "result": {"exit_code": 0, "stdout": "lots of output"}}
            ]
        })
    }

    #[test]
    fn reduce_keeps_only_name_target_and_exit_code() {
        let reduced = reduce_trace(&sample_trace()).unwrap();
        let text = reduced.to_string();
        assert!(!text.contains("private task title"));
        assert!(!text.contains("secret body"));
        assert!(!text.contains("file contents"));
        assert!(!text.contains("lots of output"));
        assert_eq!(reduced["tool_calls"][0]["input"]["target"], "src/lib.rs");
        assert_eq!(reduced["tool_calls"][1]["result"]["exit_code"], 0);
    }

    #[test]
    fn reduce_requires_tool_calls() {
        assert!(reduce_trace(&json!({"session_id": "s"})).is_err());
    }

    #[test]
    fn credential_shapes_are_detected() {
        assert!(contains_secret("token sk-abc123"));
        assert!(contains_secret("-----BEGIN PRIVATE KEY-----"));
        assert!(!contains_secret("cargo test --lib"));
    }

    #[test]
    fn capture_needs_both_flags_and_a_passing_verify() {
        let mut settings = AgentBrainSettings::default();
        assert!(capture_allowed(&settings, true).is_err());
        enable(&mut settings, false);
        assert!(capture_allowed(&settings, true).is_err());
        enable(&mut settings, true);
        assert!(capture_allowed(&settings, false).is_err());
        assert!(capture_allowed(&settings, true).is_ok());
    }

    #[test]
    fn enable_without_capture_leaves_capture_off() {
        let mut settings = AgentBrainSettings::default();
        enable(&mut settings, false);
        assert!(settings.memorable.enabled);
        assert!(!settings.memorable.capture);
    }

    #[test]
    fn memorable_is_off_by_default() {
        let settings = AgentBrainSettings::default();
        assert!(!settings.memorable.enabled);
        assert!(!settings.memorable.capture);
        assert!(!is_registered(&settings));
    }

    #[test]
    fn enable_registers_a_read_only_server_pointed_at_an_unreachable_api() {
        let mut settings = AgentBrainSettings::default();
        enable(&mut settings, false);
        let server = settings
            .upstream_mcp
            .servers
            .iter()
            .find(|s| s.name == SERVER_NAME)
            .unwrap();
        assert_eq!(server.command, "memorable");
        assert_eq!(server.args, vec!["mcp".to_string()]);
        assert_eq!(
            server.env.get("MEMORABLE_API_URL").map(String::as_str),
            Some(UNREACHABLE_API_URL)
        );
    }

    #[test]
    fn enable_twice_registers_one_server() {
        let mut settings = AgentBrainSettings::default();
        enable(&mut settings, false);
        enable(&mut settings, true);
        assert_eq!(
            settings
                .upstream_mcp
                .servers
                .iter()
                .filter(|s| s.name == SERVER_NAME)
                .count(),
            1
        );
    }

    #[test]
    fn disable_clears_capture_and_keeps_other_servers() {
        let mut settings = AgentBrainSettings::default();
        settings.upstream_mcp.servers.push(UpstreamServerConfig {
            name: "stitch".to_string(),
            command: "npx".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            enabled: true,
        });
        enable(&mut settings, true);
        disable(&mut settings);
        assert!(!settings.memorable.enabled);
        assert!(!settings.memorable.capture);
        assert!(!is_registered(&settings));
        assert!(settings
            .upstream_mcp
            .servers
            .iter()
            .any(|s| s.name == "stitch"));
    }
}
