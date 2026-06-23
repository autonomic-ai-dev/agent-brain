use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_NAMES: &[&str] = &["config.yaml", "config.yml", "config.json"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AgentBrainSettings {
    #[serde(default)]
    pub auto_update: AutoUpdateSettings,
    #[serde(default)]
    pub sync: SyncSettings,
    #[serde(default)]
    pub upstream_mcp: UpstreamMcpSettings,
    #[serde(default)]
    pub memory_gc: MemoryGcSettings,
    #[serde(default)]
    pub observation: ObservationSettings,
    #[serde(default)]
    pub trace_extract: TraceExtractSettings,
    #[serde(default)]
    pub graphify: GraphifySettings,
    #[serde(default)]
    pub registry: RegistrySettings,
    #[serde(default)]
    pub docs: DocsSettings,
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
pub struct ObservationSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_observation_min_facts")]
    pub min_facts_per_topic: usize,
    #[serde(default = "default_observation_window_days")]
    pub window_days: u32,
}

fn default_observation_min_facts() -> usize {
    3
}

fn default_observation_window_days() -> u32 {
    90
}

impl Default for ObservationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_facts_per_topic: default_observation_min_facts(),
            window_days: default_observation_window_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceExtractSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_trace_extract_confidence")]
    pub confidence: f64,
}

fn default_trace_extract_confidence() -> f64 {
    0.75
}

impl Default for TraceExtractSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            confidence: default_trace_extract_confidence(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphifySettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_graphify_bin")]
    pub graphify_bin: String,
    #[serde(default = "default_graphify_max_jobs")]
    pub max_concurrent_jobs: usize,
    #[serde(default = "default_graphify_query_budget")]
    pub default_query_budget: usize,
}

fn default_graphify_bin() -> String {
    "graphify".into()
}

fn default_graphify_max_jobs() -> usize {
    1
}

fn default_graphify_query_budget() -> usize {
    1500
}

impl Default for GraphifySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            graphify_bin: default_graphify_bin(),
            max_concurrent_jobs: default_graphify_max_jobs(),
            default_query_budget: default_graphify_query_budget(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocsSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_docs_allowed_domains")]
    pub allowed_domains: Vec<String>,
    #[serde(default = "default_docs_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_docs_max_chunks")]
    pub max_chunks: usize,
    #[serde(default = "default_docs_chunk_words")]
    pub chunk_words: usize,
    #[serde(default = "default_docs_summary_words")]
    pub summary_words: usize,
}

fn default_docs_allowed_domains() -> Vec<String> {
    vec![
        "nextjs.org".into(),
        "react.dev".into(),
        "docs.vercel.com".into(),
        "doc.rust-lang.org".into(),
        "docs.rs".into(),
        "tailwindcss.com".into(),
        "docs.cursor.com".into(),
        "developer.mozilla.org".into(),
        "typescriptlang.org".into(),
        "nodejs.org".into(),
        "docs.github.com".into(),
    ]
}

fn default_docs_max_bytes() -> usize {
    512_000
}

fn default_docs_max_chunks() -> usize {
    6
}

fn default_docs_chunk_words() -> usize {
    350
}

fn default_docs_summary_words() -> usize {
    45
}

impl Default for DocsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_domains: default_docs_allowed_domains(),
            max_bytes: default_docs_max_bytes(),
            max_chunks: default_docs_max_chunks(),
            chunk_words: default_docs_chunk_words(),
            summary_words: default_docs_summary_words(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistrySettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// GitHub repo slug for raw registry fetch (future: autonomic-ai-dev/agent-registry).
    #[serde(default = "default_registry_remote_repo")]
    pub remote_repo: String,
    #[serde(default = "default_registry_remote_ref")]
    pub remote_ref: String,
    /// Path inside the remote repo (interim: agent-brain ships registry under this prefix).
    #[serde(default = "default_registry_subpath")]
    pub registry_subpath: String,
    #[serde(default = "default_registry_sync_interval_hours")]
    pub sync_interval_hours: u32,
    #[serde(default = "default_true")]
    pub sync_on_doctor: bool,
}

fn default_registry_remote_repo() -> String {
    "autonomic-ai-dev/agent-registry".into()
}

fn default_registry_remote_ref() -> String {
    "master".into()
}

fn default_registry_subpath() -> String {
    String::new()
}

fn default_registry_sync_interval_hours() -> u32 {
    24
}

impl Default for RegistrySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            remote_repo: default_registry_remote_repo(),
            remote_ref: default_registry_remote_ref(),
            registry_subpath: default_registry_subpath(),
            sync_interval_hours: default_registry_sync_interval_hours(),
            sync_on_doctor: true,
        }
    }
}

impl RegistrySettings {
    pub fn raw_url_base(&self) -> String {
        let sub = self.registry_subpath.trim_matches('/');
        if sub.is_empty() {
            format!(
                "https://raw.githubusercontent.com/{}/{}",
                self.remote_repo.trim(),
                self.remote_ref.trim()
            )
        } else {
            format!(
                "https://raw.githubusercontent.com/{}/{}/{}",
                self.remote_repo.trim(),
                self.remote_ref.trim(),
                sub
            )
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
        let _ = agent_body_core::run_legacy_migrations();
        let mut settings = Self::from_unified_config()
            .or_else(|| Self::from_file(home).ok())
            .unwrap_or_default();
        settings.apply_env_overrides();
        settings
    }

    fn from_unified_config() -> Option<Self> {
        let table = agent_body_core::read_organ_section_raw("brain").ok()??;
        if table.is_empty() {
            return None;
        }
        let json = serde_json::to_value(&table).ok()?;
        serde_json::from_value(json).ok()
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
        if std::env::var("AGENT_BRAIN_HOME").is_ok() {
            let path = home.join("config.yaml");
            if path.exists() {
                return Ok(path);
            }
            fs::create_dir_all(home).context("create home dir")?;
            let settings = Self::default_enabled_example();
            let yaml = serde_yaml::to_string(&settings).context("serialize default config")?;
            fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
            return Ok(path);
        }

        agent_body_core::ensure_default_ecosystem_sections().context("init ecosystem config")?;
        if let Some(existing) = agent_body_core::read_organ_section_raw("brain")? {
            if !existing.is_empty() {
                return Ok(agent_body_core::config_path());
            }
        }
        let settings = Self::default_enabled_example();
        let json = serde_json::to_value(&settings).context("serialize [brain]")?;
        let table: toml::Table = serde_json::from_value(json).context("convert [brain] to toml")?;
        agent_body_core::write_organ_section_raw("brain", &table)?;
        Ok(agent_body_core::config_path())
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
            observation: ObservationSettings::default(),
            trace_extract: TraceExtractSettings::default(),
            graphify: GraphifySettings::default(),
            registry: RegistrySettings::default(),
            docs: DocsSettings::default(),
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
        .map(|v| {
            matches!(
                v.to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
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
