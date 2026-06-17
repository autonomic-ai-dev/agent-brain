use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

mod curated;
pub use curated::{list_aliases, resolve_package_inputs, CuratedAliasInfo};

const REGISTRY_FILE: &str = "packages.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageRecord {
    pub name: String,
    pub source: String,
    pub git_url: String,
    pub git_ref: String,
    pub install_path: String,
    pub commit: Option<String>,
    pub installed_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageRegistry {
    pub packages: Vec<PackageRecord>,
}

impl PackageRegistry {
    pub fn load(home: &Path) -> Result<Self> {
        let path = registry_path(home);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).context("parse packages.json")
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let path = registry_path(home);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let pretty = serde_json::to_string_pretty(self)?;
        fs::write(path, format!("{pretty}\n")).context("write packages.json")
    }

    pub fn get(&self, name: &str) -> Option<&PackageRecord> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn remove(&mut self, name: &str) -> Option<PackageRecord> {
        if let Some(idx) = self.packages.iter().position(|p| p.name == name) {
            Some(self.packages.remove(idx))
        } else {
            None
        }
    }
}

fn registry_path(home: &Path) -> PathBuf {
    home.join(REGISTRY_FILE)
}

pub fn packages_dir(home: &Path) -> PathBuf {
    home.join("packages")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSource {
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
}

pub fn parse_source(input: &str) -> Result<PackageSource> {
    let trimmed = input.trim().trim_end_matches('/').trim_end_matches(".git");
    let git_ref = "main".to_string();

    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        return parse_owner_repo(rest, git_ref);
    }
    if let Some(rest) = trimmed.strip_prefix("http://github.com/") {
        return parse_owner_repo(rest, git_ref);
    }
    if let Some(rest) = trimmed.strip_prefix("github.com/") {
        return parse_owner_repo(rest, git_ref);
    }
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return parse_owner_repo(rest, git_ref);
    }
    if trimmed.contains('/') && !trimmed.contains(':') && !trimmed.contains(' ') {
        return parse_owner_repo(trimmed, git_ref);
    }

    bail!("unsupported package source: {input}. Use owner/repo or a GitHub URL");
}

fn parse_owner_repo(value: &str, git_ref: String) -> Result<PackageSource> {
    let mut parts = value.split('/');
    let owner = parts
        .next()
        .filter(|s| !s.is_empty())
        .context("missing GitHub owner")?
        .to_string();
    let repo = parts
        .next()
        .filter(|s| !s.is_empty())
        .context("missing GitHub repo name")?
        .to_string();
    Ok(PackageSource {
        owner,
        repo,
        git_ref,
    })
}

pub fn package_name(source: &PackageSource) -> String {
    source.repo.to_ascii_lowercase()
}

pub fn git_url(source: &PackageSource) -> String {
    format!(
        "https://github.com/{}/{}.git",
        source.owner, source.repo
    )
}

pub fn add_package(config: &Config, source_input: &str, git_ref: Option<&str>) -> Result<PackageRecord> {
    config.ensure_dirs()?;
    let mut source = parse_source(source_input)?;
    if let Some(r) = git_ref {
        source.git_ref = r.to_string();
    }

    let name = package_name(&source);
    let install_dir = packages_dir(&config.home).join(&name);
    let git_url = git_url(&source);

    if install_dir.exists() {
        update_package_at(&install_dir, &source.git_ref)?;
    } else {
        clone_package(&git_url, &install_dir, &source.git_ref)?;
    }

    let commit = git_head(&install_dir).ok();
    let record = PackageRecord {
        name: name.clone(),
        source: format!("{}/{}", source.owner, source.repo),
        git_url,
        git_ref: source.git_ref.clone(),
        install_path: install_dir.display().to_string(),
        commit,
        installed_at: chrono::Utc::now().timestamp(),
    };

    let mut registry = PackageRegistry::load(&config.home)?;
    registry.remove(&name);
    registry.packages.push(record.clone());
    registry.save(&config.home)?;

    if name == "starter" || source_input.trim().trim_start_matches('@') == "starter" {
        if let Ok(store) = crate::db::store::BrainStore::open(&config.db_path) {
            let _ = crate::adoption::record_starter_pack(&store);
        }
    }

    Ok(record)
}

pub fn remove_package(config: &Config, name: &str) -> Result<u64> {
    let mut registry = PackageRegistry::load(&config.home)?;
    let record = registry
        .remove(name)
        .with_context(|| format!("package '{name}' is not installed"))?;
    registry.save(&config.home)?;

    let purged = purge_package_index(config, &record.install_path)?;

    let path = PathBuf::from(&record.install_path);
    if path.exists() {
        fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(purged)
}

fn purge_package_index(config: &Config, install_path: &str) -> Result<u64> {
    let store = crate::db::store::BrainStore::open(&config.db_path)?;
    let n = store.delete_indexed_items_under_prefix(install_path)?;
    if n > 0 {
        store.bump_index_version()?;
    }
    Ok(n)
}

pub fn update_packages(config: &Config, name: Option<&str>) -> Result<Vec<PackageRecord>> {
    let mut registry = PackageRegistry::load(&config.home)?;
    let targets: Vec<PackageRecord> = if let Some(name) = name {
        vec![registry
            .get(name)
            .cloned()
            .with_context(|| format!("package '{name}' is not installed"))?]
    } else {
        registry.packages.clone()
    };

    let mut updated = Vec::new();
    for pkg in targets {
        let path = PathBuf::from(&pkg.install_path);
        update_package_at(&path, &pkg.git_ref)?;
        let mut next = pkg.clone();
        next.commit = git_head(&path).ok();
        next.installed_at = chrono::Utc::now().timestamp();
        registry.remove(&next.name);
        registry.packages.push(next.clone());
        updated.push(next);
    }

    registry.save(&config.home)?;
    Ok(updated)
}

pub fn list_packages(config: &Config) -> Result<Vec<PackageRecord>> {
    Ok(PackageRegistry::load(&config.home)?.packages)
}

pub fn package_index_roots(home: &Path) -> Vec<PathBuf> {
    let registry = PackageRegistry::load(home).unwrap_or_default();
    let mut roots = Vec::new();
    for pkg in registry.packages {
        let path = PathBuf::from(pkg.install_path);
        if path.exists() {
            roots.extend(discover_package_roots(&path));
        }
    }
    roots
}

pub fn discover_package_roots(package_root: &Path) -> Vec<PathBuf> {
    if let Some(manifest_roots) = read_manifest_roots(package_root) {
        return manifest_roots
            .into_iter()
            .map(|rel| package_root.join(rel))
            .filter(|p| p.exists())
            .collect();
    }

    let mut roots = Vec::new();
    for rel in [
        "skills",
        "agents",
        "rules",
        "commands",
        ".cursor/rules",
        ".cursor/skills",
        ".cursor/skills-cursor",
        ".claude/skills",
        ".claude/agents",
        ".codex/skills",
        ".codex/agents",
    ] {
        let path = package_root.join(rel);
        if path.exists() {
            roots.push(path);
        }
    }

    for file in ["AGENTS.md", "CLAUDE.md", "RULES.md", ".cursorrules"] {
        let path = package_root.join(file);
        if path.is_file() {
            roots.push(path);
        }
    }

    roots
}

fn read_manifest_roots(package_root: &Path) -> Option<Vec<String>> {
    for name in ["agent-brain.yaml", "agent-brain.yml", ".agent-brain.yaml"] {
        let path = package_root.join(name);
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path).ok()?;
        let manifest: PackageManifest = serde_yaml::from_str(&raw).ok()?;
        if !manifest.roots.is_empty() {
            return Some(manifest.roots);
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct PackageManifest {
    roots: Vec<String>,
}

fn clone_package(url: &str, dest: &Path, git_ref: &str) -> Result<()> {
    if dest.exists() {
        bail!("install path already exists: {}", dest.display());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let status = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            git_ref,
            url,
            &dest.display().to_string(),
        ])
        .status()
        .context("spawn git clone")?;

    if !status.success() {
        // Retry without branch in case default branch differs.
        if dest.exists() {
            fs::remove_dir_all(dest).ok();
        }
        let fallback = Command::new("git")
            .args(["clone", "--depth", "1", url, &dest.display().to_string()])
            .status()
            .context("spawn git clone fallback")?;
        if !fallback.success() {
            bail!("git clone failed for {url}");
        }
    }
    Ok(())
}

fn update_package_at(path: &Path, git_ref: &str) -> Result<()> {
    if !path.join(".git").exists() {
        bail!("{} is not a git checkout", path.display());
    }

    let fetch = Command::new("git")
        .args(["fetch", "--depth", "1", "origin", git_ref])
        .current_dir(path)
        .status()
        .context("spawn git fetch")?;
    if !fetch.success() {
        Command::new("git")
            .args(["fetch", "--depth", "1", "origin"])
            .current_dir(path)
            .status()
            .context("spawn git fetch origin")?;
    }

    let reset = Command::new("git")
        .args(["reset", "--hard", &format!("origin/{git_ref}")])
        .current_dir(path)
        .status()
        .context("spawn git reset")?;
    if !reset.success() {
        Command::new("git")
            .args(["reset", "--hard", "FETCH_HEAD"])
            .current_dir(path)
            .status()
            .context("spawn git reset FETCH_HEAD")?;
    }
    Ok(())
}

fn git_head(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .context("spawn git rev-parse")?;
    if !output.status.success() {
        bail!("git rev-parse failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shorthand_source() {
        let src = parse_source("affaan-m/ecc").unwrap();
        assert_eq!(src.owner, "affaan-m");
        assert_eq!(src.repo, "ecc");
    }

    #[test]
    fn parses_github_url() {
        let src = parse_source("https://github.com/affaan-m/ecc").unwrap();
        assert_eq!(src.owner, "affaan-m");
        assert_eq!(src.repo, "ecc");
    }

    #[test]
    fn discovers_standard_package_roots() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("skills/foo")).unwrap();
        fs::create_dir_all(dir.path().join("agents")).unwrap();
        fs::write(dir.path().join("AGENTS.md"), "# agents").unwrap();

        let roots = discover_package_roots(dir.path());
        assert!(roots.iter().any(|p| p.ends_with("skills")));
        assert!(roots.iter().any(|p| p.ends_with("agents")));
        assert!(roots.iter().any(|p| p.ends_with("AGENTS.md")));
    }
}
