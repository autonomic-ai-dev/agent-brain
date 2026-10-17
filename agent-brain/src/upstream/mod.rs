//! Upstream MCP federation: registry, tool index, routing, semantic truncation.

mod client;
mod suggest;
mod truncate;

pub use client::{
    call_tool_result_to_text, call_upstream_tool, list_server_tools, refresh_upstream_index,
    refresh_upstream_index_blocking, UpstreamCallLog,
};
pub use suggest::suggest_upstream_tools;
pub use truncate::truncate_upstream_result;

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::secrets;
use crate::settings::{UpstreamMcpSettings, UpstreamServerConfig};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexedUpstreamTool {
    pub server: String,
    pub name: String,
    pub description: String,
}

pub fn enabled_servers(settings: &UpstreamMcpSettings) -> Vec<&UpstreamServerConfig> {
    if !settings.enabled {
        return Vec::new();
    }
    settings
        .servers
        .iter()
        .filter(|s| s.enabled)
        .take(settings.max_servers.max(1))
        .collect()
}

pub fn find_server<'a>(
    settings: &'a UpstreamMcpSettings,
    name: &str,
) -> Option<&'a UpstreamServerConfig> {
    enabled_servers(settings)
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
}

pub fn resolve_env_map(env: &HashMap<String, String>) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for (key, value) in env {
        out.insert(key.clone(), resolve_env_value(value)?);
    }
    Ok(out)
}

pub fn resolve_env_value(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        return secrets::get_secret(inner)?
            .with_context(|| format!("missing secret {inner} for upstream MCP env"));
    }
    Ok(trimmed.to_string())
}

pub fn secret_names_from_env(env: &HashMap<String, String>) -> Vec<String> {
    let mut names = Vec::new();
    for value in env.values() {
        let trimmed = value.trim();
        if let Some(inner) = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
            names.push(inner.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

pub fn validate_server_config(server: &UpstreamServerConfig) -> Result<()> {
    if server.name.trim().is_empty() {
        bail!("upstream server name is required");
    }
    if server.command.trim().is_empty() {
        bail!("upstream server {} missing command", server.name);
    }
    Ok(())
}
