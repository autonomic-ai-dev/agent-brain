use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct InstallMeta {
    pub binary_path: String,
    pub version: String,
    pub installed_at: String,
}

const INSTALL_META_DIR: &str = ".config/agent-brain";
const INSTALL_META_FILE: &str = "install_meta.json";

fn install_meta_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(&home).join(INSTALL_META_DIR).join(INSTALL_META_FILE)
}

fn read_meta() -> Result<Option<InstallMeta>> {
    let path = install_meta_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&content)?))
}

fn write_meta(meta: &InstallMeta) -> Result<()> {
    let path = install_meta_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(meta)?)?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct McpConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_servers: Option<HashMap<String, McpServerConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    servers: Option<HashMap<String, McpServerConfig>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpServerConfig {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
}

fn patch_mcp_config(config_path: &Path, new_binary: &str) -> Result<bool> {
    if !config_path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(config_path)?;
    let mut config: McpConfig = serde_json::from_str(&content)?;

    let mut changed = false;
    let servers = config.mcp_servers.as_mut()
        .or(config.servers.as_mut());

    if let Some(servers) = servers {
        for (_name, server) in servers.iter_mut() {
            if server.command != new_binary {
                tracing::info!("Patching MCP server command: {} -> {}", server.command, new_binary);
                server.command = new_binary.to_string();
                changed = true;
            }
        }
    }

    if changed {
        let backup = config_path.with_extension("agent-brain.bak.json");
        std::fs::copy(config_path, &backup)?;
        std::fs::write(config_path, serde_json::to_string_pretty(&config)?)?;
        tracing::info!("Patched MCP config at {}, backup at {:?}", config_path.display(), backup);
    }

    Ok(changed)
}

const MCP_CONFIG_PATHS: &[&str] = &[
    "claude_desktop_config.json",
    ".cursor/mcp.json",
    ".codex/config.toml",
];

pub fn auto_wire(binary_path: &str, version: &str) -> Result<()> {
    let new_meta = InstallMeta {
        binary_path: binary_path.to_string(),
        version: version.to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };

    let should_update = match read_meta()? {
        Some(existing) => existing.binary_path != binary_path,
        None => true,
    };

    if !should_update {
        return Ok(());
    }

    let home = std::env::var("HOME")?;
    for rel_path in MCP_CONFIG_PATHS {
        let abs_path = PathBuf::from(&home).join(rel_path);
        patch_mcp_config(&abs_path, binary_path)?;
    }

    write_meta(&new_meta)?;
    tracing::info!("Auto-wire complete: {}", binary_path);
    Ok(())
}
