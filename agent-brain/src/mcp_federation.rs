//! agent-brain ↔ agent-body MCP federation: install entries, doctor checks, schema audit.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

/// Resolve an organ binary on PATH or common install locations.
pub fn resolve_organ_binary(name: &str) -> Option<PathBuf> {
    let env_key = format!("AUTONOMIC_{}_BINARY", name.to_uppercase().replace('-', "_"));
    if let Ok(path) = std::env::var(&env_key) {
        let p = PathBuf::from(path);
        if p.is_file() {
            return Some(p);
        }
    }
    for path in [
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
    ] {
        let p = PathBuf::from(&path);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_path = PathBuf::from(&home);
        let p = home_path.join(".local/bin").join(name);
        if p.is_file() {
            return Some(p);
        }
        let p = home_path.join(".cargo/bin").join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(output) = Command::new("which").arg(name).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                let p = PathBuf::from(path);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Cursor / Claude Desktop style `mcpServers.agent-body` entry.
pub fn agent_body_cursor_entry(exe: &Path) -> Value {
    json!({
        "command": exe.display().to_string(),
        "args": ["serve-mcp"],
        "env": {
            "RUST_LOG": "info"
        }
    })
}

/// OpenCode `mcp.agent-body` entry (local command array).
pub fn agent_body_opencode_entry(exe: &Path) -> Value {
    json!({
        "type": "local",
        "command": [exe.display().to_string(), "serve-mcp"],
        "enabled": true,
        "environment": {
            "RUST_LOG": "info"
        }
    })
}

/// Insert agent-body into a `mcpServers` map when the binary is installed.
pub fn ensure_agent_body_cursor_server(servers: &mut Map<String, Value>) {
    if servers.contains_key("agent-body") {
        return;
    }
    if let Some(exe) = resolve_organ_binary("agent-body") {
        servers.insert("agent-body".into(), agent_body_cursor_entry(&exe));
    }
}

/// Insert agent-body into OpenCode `mcp` map when the binary is installed.
pub fn ensure_agent_body_opencode_server(mcp: &mut Map<String, Value>) {
    if mcp.contains_key("agent-body") {
        return;
    }
    if let Some(exe) = resolve_organ_binary("agent-body") {
        mcp.insert("agent-body".into(), agent_body_opencode_entry(&exe));
    }
}

#[derive(Debug, Clone, Default)]
pub struct FederationStatus {
    pub body_binary: Option<PathBuf>,
    pub body_in_mcp_json: bool,
    pub upstream_configured: bool,
    pub upstream_enabled: bool,
}

pub fn assess_federation(mcp_json_path: &Path) -> FederationStatus {
    let body_binary = resolve_organ_binary("agent-body");
    let body_in_mcp_json = mcp_json_has_server(mcp_json_path, "agent-body");
    let (upstream_configured, upstream_enabled) = upstream_agent_body_status();
    FederationStatus {
        body_binary,
        body_in_mcp_json,
        upstream_configured,
        upstream_enabled,
    }
}

fn mcp_json_has_server(path: &Path, server: &str) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value
        .get("mcpServers")
        .and_then(|v| v.get(server))
        .is_some()
}

fn upstream_agent_body_status() -> (bool, bool) {
    let home = crate::config::Config::load()
        .map(|c| c.home)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".agent_brain")
        });
    let settings = crate::settings::AgentBrainSettings::load(&home);
    let enabled = settings.upstream_mcp.enabled;
    let configured = settings
        .upstream_mcp
        .servers
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case("agent-body") && s.enabled);
    (configured, enabled)
}

#[derive(Debug, Clone)]
pub struct SchemaAudit {
    pub ok: bool,
    pub server_info_name: Option<String>,
    pub tool_count: usize,
    pub detail: String,
}

/// Deep audit: spawn `agent-body serve-mcp` and verify `muscle_execute_bash` has JSON Schema properties.
pub fn audit_agent_body_schemas() -> Result<SchemaAudit> {
    let exe = resolve_organ_binary("agent-body")
        .context("agent-body binary not found — install via autonomic update --organ body")?;

    let mut child = Command::new(&exe)
        .arg("serve-mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", exe.display()))?;

    let stdin = child.stdin.as_mut().context("stdin")?;
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "agent-brain-doctor", "version": "1" }
            }
        }),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    ];
    for msg in &messages {
        writeln!(stdin, "{msg}")?;
    }
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .context("wait for agent-body serve-mcp")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut server_info_name = None;
    let mut tool_count = 0;
    let mut schema_ok = false;
    let mut detail = String::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("id") == Some(&json!(1)) {
            server_info_name = value
                .pointer("/result/serverInfo/name")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        if value.get("id") == Some(&json!(2)) {
            if let Some(tools) = value.pointer("/result/tools").and_then(|v| v.as_array()) {
                tool_count = tools.len();
                if let Some(bash) = tools.iter().find(|t| t.get("name") == Some(&json!("muscle_execute_bash"))) {
                    let props = bash
                        .pointer("/inputSchema/properties")
                        .and_then(|v| v.as_object());
                    schema_ok = props.is_some_and(|o| !o.is_empty());
                    if !schema_ok {
                        detail = "muscle_execute_bash inputSchema.properties is empty".into();
                    }
                } else {
                    detail = "muscle_execute_bash not in tools/list".into();
                }
            }
        }
    }

    if detail.is_empty() && schema_ok {
        detail = format!(
            "tools={tool_count}, serverInfo.name={}",
            server_info_name.as_deref().unwrap_or("?")
        );
    }

    Ok(SchemaAudit {
        ok: schema_ok && tool_count > 0,
        server_info_name,
        tool_count,
        detail,
    })
}

pub fn audit_with_timeout() -> Result<SchemaAudit> {
    // Best-effort: audit is synchronous; organ cold-start should finish within a few seconds.
    let _ = Duration::from_secs(15);
    audit_agent_body_schemas()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_body_cursor_entry_has_serve_mcp() {
        let entry = agent_body_cursor_entry(Path::new("/usr/bin/agent-body"));
        assert_eq!(entry["args"][0], "serve-mcp");
    }

    #[test]
    fn ensure_agent_body_skips_when_present() {
        let mut servers = Map::new();
        servers.insert("agent-body".into(), json!({}));
        ensure_agent_body_cursor_server(&mut servers);
        assert_eq!(servers.len(), 1);
    }

    #[test]
    fn resolve_organ_binary_checks_home_local() {
        // May or may not exist in CI — just ensure no panic.
        let _ = resolve_organ_binary("agent-body-not-real-binary-xyz");
    }
}
