//! Registry cache + sync — embedded snapshot now, central repo later.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::settings::RegistrySettings;

const EMBEDDED_FILES: &[(&str, &str)] = &[
    (
        "manifest.json",
        include_str!("../registry/manifest.json"),
    ),
    (
        "packages.json",
        include_str!("../registry/packages.json"),
    ),
    (
        "utilities.json",
        include_str!("../registry/utilities.json"),
    ),
    (
        "workflows.json",
        include_str!("../registry/workflows.json"),
    ),
    (
        "workflows/release-notes.yaml",
        include_str!("../registry/workflows/release-notes.yaml"),
    ),
    (
        "workflows/stacked-pr.yaml",
        include_str!("../registry/workflows/stacked-pr.yaml"),
    ),
    (
        "workflows/bugfix.yaml",
        include_str!("../registry/workflows/bugfix.yaml"),
    ),
    (
        "workflows/mcp-health-check.yaml",
        include_str!("../registry/workflows/mcp-health-check.yaml"),
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMeta {
    source: String,
    remote_repo: String,
    remote_ref: String,
    synced_at_ms: i64,
    files: usize,
}

pub fn cache_dir(home: &Path) -> PathBuf {
    home.join("registry-cache")
}

/// Read registry JSON/YAML from cache when present, else compile-time embedded copy.
pub fn read_registry_file(home: &Path, relative: &str, embedded: &str) -> String {
    let path = cache_dir(home).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|_| embedded.to_string())
}

/// Copy embedded registry into `~/.agent_brain/registry-cache/`.
pub fn seed_embedded(home: &Path) -> Result<usize> {
    let n = write_files(home, EMBEDDED_FILES, "embedded")?;
    write_meta(
        home,
        SyncMeta {
            source: "embedded".into(),
            remote_repo: String::new(),
            remote_ref: String::new(),
            synced_at_ms: now_ms(),
            files: n,
        },
    )?;
    Ok(n)
}

/// Fetch registry files from configured GitHub raw URL (interim: agent-brain repo path).
pub fn sync_remote(home: &Path, settings: &RegistrySettings) -> Result<usize> {
    let base = settings.raw_url_base();
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();
    let mut files: Vec<(&str, String)> = Vec::new();
    for (rel, _) in EMBEDDED_FILES {
        let url = format!("{base}/{rel}");
        let body = agent
            .get(&url)
            .call()
            .with_context(|| format!("fetch registry file {url}"))?
            .body_mut()
            .read_to_string()
            .with_context(|| format!("read registry body {rel}"))?;
        files.push((rel, body));
    }
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(r, b)| (*r, b.as_str())).collect();
    let n = write_files(home, &borrowed, "remote")?;
    write_meta(
        home,
        SyncMeta {
            source: "remote".into(),
            remote_repo: settings.remote_repo.clone(),
            remote_ref: settings.remote_ref.clone(),
            synced_at_ms: now_ms(),
            files: n,
        },
    )?;
    Ok(n)
}

pub fn sync(home: &Path, settings: &RegistrySettings) -> Result<usize> {
    if settings.remote_repo.trim().is_empty() {
        return seed_embedded(home);
    }
    match sync_remote(home, settings) {
        Ok(n) => Ok(n),
        Err(err) => {
            tracing::warn!(error = %err, "registry remote sync failed; seeding embedded snapshot");
            seed_embedded(home)
        }
    }
}

/// Seed cache when empty; optionally refresh when stale per settings.
pub fn ensure_cached(home: &Path, settings: &RegistrySettings) -> Result<()> {
    let manifest = cache_dir(home).join("manifest.json");
    if !manifest.is_file() {
        seed_embedded(home)?;
        return Ok(());
    }
    if !settings.enabled {
        return Ok(());
    }
    if settings.sync_on_doctor && is_stale(home, settings.sync_interval_hours) {
        let _ = sync(home, settings);
    }
    Ok(())
}

fn is_stale(home: &Path, interval_hours: u32) -> bool {
    let Ok(raw) = fs::read_to_string(cache_dir(home).join("sync-meta.json")) else {
        return true;
    };
    let Ok(meta) = serde_json::from_str::<SyncMeta>(&raw) else {
        return true;
    };
    let age_ms = now_ms().saturating_sub(meta.synced_at_ms);
    age_ms > i64::from(interval_hours) * 3_600_000
}

fn write_files(home: &Path, files: &[(&str, &str)], _source: &str) -> Result<usize> {
    let base = cache_dir(home);
    for (rel, content) in files {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(files.len())
}

fn write_meta(home: &Path, meta: SyncMeta) -> Result<()> {
    let path = cache_dir(home).join("sync-meta.json");
    fs::create_dir_all(cache_dir(home))?;
    fs::write(&path, serde_json::to_string_pretty(&meta)?)?;
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn seed_embedded_writes_manifest() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let n = seed_embedded(home).unwrap();
        assert!(n >= 7);
        assert!(cache_dir(home).join("manifest.json").is_file());
        assert!(cache_dir(home).join("workflows/release-notes.yaml").is_file());
    }

    #[test]
    fn read_prefers_cache_over_embedded() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        seed_embedded(home).unwrap();
        let path = cache_dir(home).join("packages.json");
        fs::write(&path, r#"{"version":99,"aliases":{}}"#).unwrap();
        let raw = read_registry_file(home, "packages.json", "{}");
        assert!(raw.contains("\"version\":99"));
    }
}
