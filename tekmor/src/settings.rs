use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_NAMES: &[&str] = &["config.yaml", "config.yml", "config.json"];

static SETTINGS_CACHE: std::sync::Mutex<
    Option<(PathBuf, std::time::SystemTime, AgentBrainSettings)>,
> = std::sync::Mutex::new(None);

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AgentBrainSettings {
    #[serde(default)]
    pub registry: RegistrySettings,
    #[serde(default)]
    pub gates: GateSettings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GateMode {
    #[default]
    Enforce,
    Watch,
}

impl GateMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GateMode::Enforce => "enforce",
            GateMode::Watch => "watch",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GateSettings {
    #[serde(default)]
    pub stop: GateMode,
}

fn stop_gate_override(value: Option<&str>) -> Option<GateMode> {
    match value {
        Some("watch") => Some(GateMode::Watch),
        Some("enforce") => Some(GateMode::Enforce),
        _ => None,
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

impl AgentBrainSettings {
    pub fn load(home: &Path) -> Self {
        let mut settings = Self::from_file(home).ok().unwrap_or_default();
        settings.apply_env_overrides();
        settings
    }

    pub fn from_file(home: &Path) -> Result<Self> {
        let path = config_path(home)?;
        let mtime = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        if let Ok(guard) = SETTINGS_CACHE.lock() {
            if let Some((cached_path, cached_mtime, cached_settings)) = guard.as_ref() {
                if *cached_path == path && *cached_mtime == mtime {
                    return Ok(cached_settings.clone());
                }
            }
        }

        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let parsed: Self = if path.extension().and_then(|e| e.to_str()) == Some("json") {
            serde_json::from_str(&raw).context("parse config.json")?
        } else {
            serde_yaml::from_str(&raw).context("parse config.yaml")?
        };

        if let Ok(mut guard) = SETTINGS_CACHE.lock() {
            *guard = Some((path, mtime, parsed.clone()));
        }

        Ok(parsed)
    }

    fn apply_env_overrides(&mut self) {
        if let Some(mode) = stop_gate_override(std::env::var("TEKMOR_STOP_GATE").ok().as_deref()) {
            self.gates.stop = mode;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_gate_env_values_other_than_watch_and_enforce_are_ignored() {
        assert_eq!(stop_gate_override(Some("watch")), Some(GateMode::Watch));
        assert_eq!(stop_gate_override(Some("enforce")), Some(GateMode::Enforce));
        assert_eq!(stop_gate_override(Some("sometimes")), None);
        assert_eq!(stop_gate_override(None), None);
    }

    #[test]
    fn stop_gate_mode_is_read_from_the_gates_section() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.yaml"), "gates:\n  stop: watch\n").unwrap();
        let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
        assert_eq!(settings.gates.stop, GateMode::Watch);
    }

    #[test]
    fn a_missing_gates_section_defaults_the_stop_gate_to_enforce() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.yaml"), "registry:\n  enabled: true\n").unwrap();
        let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
        assert_eq!(settings.gates.stop, GateMode::Enforce);
    }

    #[test]
    fn an_unknown_stop_gate_value_fails_the_whole_file_so_load_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.yaml"), "gates:\n  stop: sometimes\n").unwrap();
        assert!(AgentBrainSettings::from_file(dir.path()).is_err());
        assert_eq!(
            AgentBrainSettings::load(dir.path()).gates.stop,
            GateMode::Enforce
        );
    }
}
