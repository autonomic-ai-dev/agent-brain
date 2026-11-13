//! Upstream MCP federation: registry, tool index, routing, semantic truncation.

mod client;
mod suggest;
mod truncate;

pub use client::{
    call_tool_result_to_text, call_upstream_tool, call_upstream_tools_batch,
    get_upstream_tool_schema, list_server_tools, refresh_upstream_index,
    refresh_upstream_index_blocking, UpstreamBatchItem, UpstreamBatchOutcome, UpstreamCallLog,
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

pub fn is_over_max_servers(settings: &UpstreamMcpSettings, server: &UpstreamServerConfig) -> bool {
    settings.enabled
        && server.enabled
        && !enabled_servers(settings)
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case(&server.name))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, enabled: bool) -> UpstreamServerConfig {
        UpstreamServerConfig {
            name: name.into(),
            command: "npx".into(),
            args: vec![],
            env: HashMap::new(),
            enabled,
        }
    }

    fn settings(
        enabled: bool,
        max_servers: usize,
        servers: Vec<UpstreamServerConfig>,
    ) -> UpstreamMcpSettings {
        UpstreamMcpSettings {
            enabled,
            max_servers,
            suggest_limit: 4,
            servers,
        }
    }

    #[test]
    fn enabled_server_past_max_servers_is_over_cap() {
        let s = settings(
            true,
            2,
            vec![server("a", true), server("b", true), server("c", true)],
        );
        assert!(!is_over_max_servers(&s, &s.servers[0]));
        assert!(!is_over_max_servers(&s, &s.servers[1]));
        assert!(is_over_max_servers(&s, &s.servers[2]));
    }

    #[test]
    fn disabled_servers_and_disabled_upstream_are_not_over_cap() {
        let s = settings(true, 1, vec![server("a", true), server("b", false)]);
        assert!(!is_over_max_servers(&s, &s.servers[1]));

        let off = settings(false, 1, vec![server("a", true), server("b", true)]);
        assert!(!is_over_max_servers(&off, &off.servers[1]));
    }
}
