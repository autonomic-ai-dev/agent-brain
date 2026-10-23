//! Registry cache + sync — embedded snapshot now, central repo later.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::settings::RegistrySettings;

const EMBEDDED_FILES: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../registry/manifest.json")),
    ("packages.json", include_str!("../registry/packages.json")),
    ("utilities.json", include_str!("../registry/utilities.json")),
    ("workflows.json", include_str!("../registry/workflows.json")),
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

pub fn validate_repo_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.starts_with('-') || slug.ends_with('-') {
        bail!("invalid repo slug: '{slug}'");
    }
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        bail!("invalid repo slug format (must be owner/repo): '{slug}'");
    }
    for part in parts {
        if part.is_empty() || part.starts_with('-') || part.starts_with('.') {
            bail!("invalid component in repo slug: '{part}'");
        }
        if !part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            bail!("forbidden characters in repo slug: '{part}'");
        }
    }
    Ok(())
}

pub fn validate_git_ref(git_ref: &str) -> Result<()> {
    if git_ref.is_empty()
        || git_ref.starts_with('-')
        || git_ref.contains("..")
        || git_ref.ends_with('/')
        || git_ref.ends_with(".lock")
    {
        bail!("invalid git ref: '{git_ref}'");
    }
    if !git_ref
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/')
    {
        bail!("forbidden characters in git ref: '{git_ref}'");
    }
    Ok(())
}

pub fn sync_remote(home: &Path, settings: &RegistrySettings) -> Result<usize> {
    let repo_slug = settings.remote_repo.trim();
    validate_repo_slug(repo_slug)?;
    let git_ref = if settings.remote_ref.trim().is_empty() {
        "master"
    } else {
        settings.remote_ref.trim()
    };
    validate_git_ref(git_ref)?;
    let repo_dir = cache_dir(home).join("repo");
    let git_url = format!("https://github.com/{repo_slug}.git");

    if repo_dir.join(".git").exists() {
        let fetch = Command::new("git")
            .args(["fetch", "--depth", "1", "origin", git_ref])
            .current_dir(&repo_dir)
            .status()
            .context("git fetch registry")?;
        if !fetch.success() {
            let _ = Command::new("git")
                .args(["fetch", "--depth", "1", "origin"])
                .current_dir(&repo_dir)
                .status();
        }
        let _ = Command::new("git")
            .args(["reset", "--hard", &format!("origin/{git_ref}")])
            .current_dir(&repo_dir)
            .status();
    } else {
        if repo_dir.exists() {
            let _ = fs::remove_dir_all(&repo_dir);
        }
        if let Some(parent) = repo_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--branch",
                git_ref,
                &git_url,
                &repo_dir.display().to_string(),
            ])
            .status()
            .context("spawn git clone registry")?;
        if !status.success() {
            let fallback = Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    &git_url,
                    &repo_dir.display().to_string(),
                ])
                .status()
                .context("spawn git clone fallback")?;
            if !fallback.success() {
                bail!("git clone failed for registry {git_url}");
            }
        }
    }

    let base = cache_dir(home);
    let mut files_synced = 0;
    if repo_dir.join("registry.json").exists() {
        fs::copy(repo_dir.join("registry.json"), base.join("registry.json"))?;
        files_synced += 1;
    }
    if repo_dir.join("utilities/utilities.json").exists() {
        let _ = fs::create_dir_all(base.join("utilities"));
        fs::copy(
            repo_dir.join("utilities/utilities.json"),
            base.join("utilities.json"),
        )?;
        files_synced += 1;
    } else if repo_dir.join("utilities.json").exists() {
        fs::copy(repo_dir.join("utilities.json"), base.join("utilities.json"))?;
        files_synced += 1;
    }
    if repo_dir.join("workflows").exists() {
        let workflows_dest = base.join("workflows");
        let _ = fs::create_dir_all(&workflows_dest);
        if let Ok(entries) = fs::read_dir(repo_dir.join("workflows")) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    let dest = workflows_dest.join(entry.file_name());
                    if fs::copy(entry.path(), dest).is_ok() {
                        files_synced += 1;
                    }
                }
            }
        }
    }

    write_meta(
        home,
        SyncMeta {
            source: "remote-git".into(),
            remote_repo: settings.remote_repo.clone(),
            remote_ref: settings.remote_ref.clone(),
            synced_at_ms: now_ms(),
            files: files_synced,
        },
    )?;

    Ok(files_synced)
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
    let has_cache = cache_dir(home).join("registry.json").is_file()
        || cache_dir(home).join("manifest.json").is_file()
        || cache_dir(home).join("repo").is_dir();
    if !has_cache {
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
    fn registry_workflows_are_runnable_spine_files() {
        let spine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agent-spine");
        if !spine.join("workflows").is_dir() {
            eprintln!("skipped: sibling agent-spine checkout not found");
            return;
        }
        let registry: serde_json::Value =
            serde_json::from_str(include_str!("../registry/workflows.json")).unwrap();
        let entries = registry["workflows"].as_object().unwrap();
        assert!(!entries.is_empty());
        for (name, entry) in entries {
            let path = entry["path"].as_str().unwrap();
            assert!(
                spine.join(path).is_file(),
                "registry entry '{name}' points at missing {path}"
            );
            assert_eq!(
                entry["invoke"].as_str().unwrap(),
                format!("agent-spine run {path}"),
                "registry entry '{name}' must be runnable as written"
            );
        }
    }

    #[test]
    fn seed_embedded_writes_manifest() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let n = seed_embedded(home).unwrap();
        assert!(n >= 7);
        assert!(cache_dir(home).join("manifest.json").is_file());
        assert!(cache_dir(home)
            .join("workflows/release-notes.yaml")
            .is_file());
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

    #[test]
    fn validates_repo_slug_formats() {
        assert!(validate_repo_slug("autonomic-ai-dev/agent-registry").is_ok());
        assert!(validate_repo_slug("owner_name/repo-1.0").is_ok());
        assert!(validate_repo_slug("").is_err());
        assert!(validate_repo_slug("-bad/repo").is_err());
        assert!(validate_repo_slug("owner/-bad").is_err());
        assert!(validate_repo_slug("owner/..").is_err());
        assert!(validate_repo_slug("owner").is_err());
        assert!(validate_repo_slug("owner/repo/extra").is_err());
        assert!(validate_repo_slug("owner/repo;rm -rf").is_err());
    }

    #[test]
    fn validates_git_ref_formats() {
        assert!(validate_git_ref("master").is_ok());
        assert!(validate_git_ref("main").is_ok());
        assert!(validate_git_ref("feat/registry-sync").is_ok());
        assert!(validate_git_ref("v1.2.3").is_ok());
        assert!(validate_git_ref("").is_err());
        assert!(validate_git_ref("--upload-pack=evil").is_err());
        assert!(validate_git_ref("-b").is_err());
        assert!(validate_git_ref("refs/heads/..").is_err());
        assert!(validate_git_ref("master;touch foo").is_err());
        assert!(validate_git_ref("master/").is_err());
    }
}
