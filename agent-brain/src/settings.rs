use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_NAMES: &[&str] = &["config.yaml", "config.yml", "config.json"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AgentBrainSettings {
    #[serde(default)]
    pub auto_update: AutoUpdateSettings,
    #[serde(default)]
    pub sync: SyncSettings,
    #[serde(default)]
    pub upstream_mcp: UpstreamMcpSettings,
    #[serde(default)]
    pub memory_gc: MemoryGcSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryGcSettings {
    #[serde(default = "default_memory_gc_stale_days")]
    pub stale_days: u32,
    #[serde(default = "default_memory_gc_very_stale_days")]
    pub very_stale_days: u32,
}

fn default_memory_gc_stale_days() -> u32 {
    90
}

fn default_memory_gc_very_stale_days() -> u32 {
    180
}

impl Default for MemoryGcSettings {
    fn default() -> Self {
        Self {
            stale_days: default_memory_gc_stale_days(),
            very_stale_days: default_memory_gc_very_stale_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpstreamMcpSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_upstream_max_servers")]
    pub max_servers: usize,
    #[serde(default = "default_upstream_suggest_limit")]
    pub suggest_limit: usize,
    #[serde(default)]
    pub servers: Vec<UpstreamServerConfig>,
}

fn default_upstream_max_servers() -> usize {
    2
}

fn default_upstream_suggest_limit() -> usize {
    2
}

impl Default for UpstreamMcpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_servers: default_upstream_max_servers(),
            suggest_limit: default_upstream_suggest_limit(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpstreamServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SyncSettings {
    #[serde(default)]
    pub git: GitSyncSettings,
    #[serde(default)]
    pub cloud: CloudSyncSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudSyncSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cloud_provider")]
    pub provider: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default = "default_cloud_key")]
    pub key: String,
    #[serde(default = "default_true")]
    pub encrypt: bool,
    #[serde(default = "default_encryption_key_env")]
    pub encryption_key_env: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub auto_push: bool,
}

fn default_cloud_provider() -> String {
    "s3".into()
}

fn default_cloud_key() -> String {
    "brain-sync.tar.zst.age".into()
}

fn default_encryption_key_env() -> String {
    "AGENT_BRAIN_SYNC_KEY".into()
}

impl Default for CloudSyncSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_cloud_provider(),
            bucket: String::new(),
            key: default_cloud_key(),
            encrypt: true,
            encryption_key_env: default_encryption_key_env(),
            region: String::new(),
            endpoint: String::new(),
            auto_push: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitSyncSettings {
    /// Git remote URL (e.g. git@github.com:user/agent-brain-private.git)
    #[serde(default)]
    pub remote: String,
    #[serde(default = "default_git_branch")]
    pub branch: String,
    #[serde(default)]
    pub include_vectors: bool,
    #[serde(default)]
    pub auto_push: bool,
}

fn default_git_branch() -> String {
    "main".into()
}

impl Default for GitSyncSettings {
    fn default() -> Self {
        Self {
            remote: String::new(),
            branch: default_git_branch(),
            include_vectors: false,
            auto_push: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoUpdateSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_interval_hours")]
    pub interval_hours: u64,
    #[serde(default)]
    pub packages: PackageAutoUpdateSettings,
    #[serde(default)]
    pub mcp: McpAutoUpdateSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageAutoUpdateSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Empty = update all installed packages.
    #[serde(default)]
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpAutoUpdateSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_repo")]
    pub repo: String,
    #[serde(default = "default_bin_path")]
    pub bin_path: String,
    #[serde(default = "default_true")]
    pub refresh_cursor: bool,
    /// After downloading a new binary during `serve`, exec/exit so Cursor reconnects to the new build.
    #[serde(default = "default_true")]
    pub restart_after_update: bool,
    /// Wait until no MCP tool calls are in flight and this many seconds have passed since the last one.
    #[serde(default = "default_restart_idle_secs")]
    pub restart_idle_secs: u64,
    /// Stop waiting for idle after this many seconds and restart anyway.
    #[serde(default = "default_restart_max_wait_secs")]
    pub restart_max_wait_secs: u64,
    /// Minimum delay before the first idle check (lets the update log flush).
    #[serde(default = "default_restart_min_delay_secs")]
    pub restart_min_delay_secs: u64,
    /// While `serve` is running, re-check GitHub for a newer MCP release every N minutes (0 = only on serve start).
    #[serde(default = "default_mcp_recheck_interval_minutes")]
    pub recheck_interval_minutes: u64,
}

fn default_restart_idle_secs() -> u64 {
    10
}

fn default_restart_max_wait_secs() -> u64 {
    300
}

fn default_restart_min_delay_secs() -> u64 {
    2
}

fn default_mcp_recheck_interval_minutes() -> u64 {
    15
}

fn default_interval_hours() -> u64 {
    24
}

fn default_true() -> bool {
    true
}

fn default_repo() -> String {
    "aeswibon/agent-brain".into()
}

fn default_bin_path() -> String {
    "~/.local/bin/agent-brain".into()
}

impl Default for AutoUpdateSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: default_interval_hours(),
            packages: PackageAutoUpdateSettings::default(),
            mcp: McpAutoUpdateSettings::default(),
        }
    }
}

impl Default for PackageAutoUpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            names: Vec::new(),
        }
    }
}

impl Default for McpAutoUpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            repo: default_repo(),
            bin_path: default_bin_path(),
            refresh_cursor: true,
            restart_after_update: true,
            restart_idle_secs: default_restart_idle_secs(),
            restart_max_wait_secs: default_restart_max_wait_secs(),
            restart_min_delay_secs: default_restart_min_delay_secs(),
            recheck_interval_minutes: default_mcp_recheck_interval_minutes(),
        }
    }
}

impl AgentBrainSettings {
    pub fn load(home: &Path) -> Self {
        let mut settings = Self::from_file(home).unwrap_or_default();
        settings.apply_env_overrides();
        settings
    }

    pub fn from_file(home: &Path) -> Result<Self> {
        let path = config_path(home)?;
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            serde_json::from_str(&raw).context("parse config.json")
        } else {
            serde_yaml::from_str(&raw).context("parse config.yaml")
        }
    }

    pub fn save_default(home: &Path) -> Result<PathBuf> {
        let path = home.join("config.yaml");
        if path.exists() {
            return Ok(path);
        }
        fs::create_dir_all(home).context("create home dir")?;
        let settings = Self::default_enabled_example();
        let yaml = serde_yaml::to_string(&settings).context("serialize default config")?;
        fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }

    fn default_enabled_example() -> Self {
        Self {
            auto_update: AutoUpdateSettings {
                enabled: true,
                ..AutoUpdateSettings::default()
            },
            sync: SyncSettings::default(),
            upstream_mcp: UpstreamMcpSettings::default(),
            memory_gc: MemoryGcSettings::default(),
        }
    }

    fn apply_env_overrides(&mut self) {
        if env_disabled("AGENT_BRAIN_AUTO_UPDATE") {
            self.auto_update.enabled = false;
        } else if env_enabled("AGENT_BRAIN_AUTO_UPDATE") {
            self.auto_update.enabled = true;
        }
        if env_disabled("AGENT_BRAIN_AUTO_UPDATE_PACKAGES") {
            self.auto_update.packages.enabled = false;
        }
        if env_disabled("AGENT_BRAIN_AUTO_UPDATE_MCP") {
            self.auto_update.mcp.enabled = false;
        }
        if let Ok(hours) = std::env::var("AGENT_BRAIN_AUTO_UPDATE_INTERVAL_HOURS") {
            if let Ok(n) = hours.parse::<u64>() {
                self.auto_update.interval_hours = n.max(1);
            }
        }
    }
}

fn env_disabled(key: &str) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(false)
}

fn env_enabled(key: &str) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

pub fn config_path(home: &Path) -> Result<PathBuf> {
    for name in CONFIG_NAMES {
        let path = home.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "no config file in {} (expected one of: {})",
        home.display(),
        CONFIG_NAMES.join(", ")
    )
}

pub fn config_path_optional(home: &Path) -> Option<PathBuf> {
    CONFIG_NAMES
        .iter()
        .map(|name| home.join(name))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml_auto_update_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            r#"
auto_update:
  enabled: true
  interval_hours: 12
  packages:
    enabled: true
    names: [ecc]
  mcp:
    enabled: false
    repo: aeswibon/agent-brain
"#,
        )
        .unwrap();
        let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
        assert!(settings.auto_update.enabled);
        assert_eq!(settings.auto_update.interval_hours, 12);
        assert_eq!(settings.auto_update.packages.names, vec!["ecc"]);
        assert!(!settings.auto_update.mcp.enabled);
    }

    #[test]
    fn env_can_disable_auto_update() {
        let mut settings = AgentBrainSettings::default_enabled_example();
        std::env::set_var("AGENT_BRAIN_AUTO_UPDATE", "0");
        settings.apply_env_overrides();
        assert!(!settings.auto_update.enabled);
        std::env::remove_var("AGENT_BRAIN_AUTO_UPDATE");
    }

    #[test]
    fn parses_upstream_mcp_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            r#"
upstream_mcp:
  enabled: true
  max_servers: 2
  servers:
    - name: github
      command: npx
      args: ["-y", "@modelcontextprotocol/server-github"]
      env:
        GITHUB_PERSONAL_ACCESS_TOKEN: "${GITHUB_TOKEN}"
"#,
        )
        .unwrap();
        let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
        assert!(settings.upstream_mcp.enabled);
        assert_eq!(settings.upstream_mcp.servers.len(), 1);
        assert_eq!(settings.upstream_mcp.servers[0].name, "github");
    }

    #[test]
    fn parses_sync_git_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            r#"
sync:
  git:
    remote: git@github.com:user/agent-brain-private.git
    branch: main
    auto_push: false
"#,
        )
        .unwrap();
        let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
        assert_eq!(
            settings.sync.git.remote,
            "git@github.com:user/agent-brain-private.git"
        );
        assert_eq!(settings.sync.git.branch, "main");
        assert!(!settings.sync.git.auto_push);
    }

    #[test]
    fn parses_memory_gc_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.yaml"),
            r#"
memory_gc:
  stale_days: 45
  very_stale_days: 120
"#,
        )
        .unwrap();
        let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
        assert_eq!(settings.memory_gc.stale_days, 45);
        assert_eq!(settings.memory_gc.very_stale_days, 120);
    }
}
