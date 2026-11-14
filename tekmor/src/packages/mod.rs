use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

mod bundles;
mod curated;
pub use bundles::install_bundled;
pub use curated::{
    default_git_sources, list_aliases, list_utilities, list_workflows, lookup_alias,
    recommend_boost_by_source, resolve_package_inputs, CuratedAliasInfo, ResolvedAlias,
    UtilityInfo, WorkflowInfo,
};

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

pub fn duplicate_packages(home: &Path) -> Vec<(String, String)> {
    use sha2::Digest;
    let dir = packages_dir(home);
    let mut skill_files: std::collections::BTreeMap<String, Vec<PathBuf>> =
        std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file() || path.file_name().is_none_or(|n| n != "SKILL.md") {
            continue;
        }
        let Ok(relative) = path.strip_prefix(&dir) else {
            continue;
        };
        let Some(package) = relative.components().next() else {
            continue;
        };
        skill_files
            .entry(package.as_os_str().to_string_lossy().into_owned())
            .or_default()
            .push(path.to_path_buf());
    }
    let mut by_digest: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (package, mut files) in skill_files {
        files.sort();
        let mut hasher = sha2::Sha256::new();
        for file in &files {
            hasher.update(fs::read(file).unwrap_or_default());
        }
        let digest = format!("{:x}", hasher.finalize());
        by_digest.entry(digest).or_default().push(package);
    }
    by_digest
        .into_values()
        .filter(|names| names.len() > 1)
        .flat_map(|names| {
            names
                .windows(2)
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect::<Vec<_>>()
        })
        .collect()
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
    format!("{}-{}", source.owner, source.repo).to_ascii_lowercase()
}

pub fn git_url(source: &PackageSource) -> String {
    format!("https://github.com/{}/{}.git", source.owner, source.repo)
}

pub fn add_package(
    config: &Config,
    source_input: &str,
    git_ref: Option<&str>,
) -> Result<PackageRecord> {
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
        if pkg.source.starts_with("tekmor/bundle:") {
            continue;
        }
        let path = PathBuf::from(&pkg.install_path);
        if !path.join(".git").exists() {
            tracing::warn!(
                "{} is not a git checkout — skipping (reinstall with tekmor add to clone from git)",
                path.display()
            );
            continue;
        }
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

/// Install curated aliases marked `default: true` when missing (git clones).
pub fn bootstrap_default_packages(config: &Config) -> Result<Vec<PackageRecord>> {
    let sources = default_git_sources()?;
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let registry = PackageRegistry::load(&config.home)?;
    let mut installed = Vec::new();
    for src in sources {
        if registry.packages.iter().any(|p| p.source == src) {
            continue;
        }
        match add_package(config, &src, None) {
            Ok(record) => {
                tracing::info!(target: "tekmor::packages", source = %src, "installed default package");
                installed.push(record);
            }
            Err(err) => {
                tracing::warn!(
                    target: "tekmor::packages",
                    source = %src,
                    error = %err,
                    "default package install failed"
                );
            }
        }
    }
    Ok(installed)
}

/// `(install_path, boost)` for installed packages that have a curated `recommend_boost`.
pub fn recommend_path_boosts(home: &Path) -> Vec<(String, f64)> {
    let Ok(by_source) = recommend_boost_by_source() else {
        return Vec::new();
    };
    if by_source.is_empty() {
        return Vec::new();
    }
    let registry = PackageRegistry::load(home).unwrap_or_default();
    let mut out = Vec::new();
    for pkg in registry.packages {
        if let Some(&boost) = by_source
            .get(&pkg.source)
            .or_else(|| by_source.get(&pkg.name))
        {
            if boost > 0.0 {
                out.push((pkg.install_path, boost));
            }
        }
    }
    out
}

/// Additive boost for an indexed item path given preferred package install prefixes.
pub fn path_recommend_boost(source_path: &str, boosts: &[(String, f64)]) -> f64 {
    let mut best = 0.0;
    for (prefix, boost) in boosts {
        if !prefix.is_empty() && source_path.starts_with(prefix.as_str()) && *boost > best {
            best = *boost;
        }
    }
    best
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
        ".claude/skills",
        ".claude/agents",
        ".claude/rules",
        ".claude/commands",
        ".agents/skills",
        ".agents/rules",
        ".agents",
        ".cursor/rules",
        ".cursor/skills",
        ".github/skills",
        ".antigravity/skills",
        ".antigravity/rules",
    ] {
        let path = package_root.join(rel);
        if path.exists() {
            roots.push(path);
        }
    }

    for file in ["AGENTS.md", "RULES.md"] {
        let path = package_root.join(file);
        if path.is_file() {
            roots.push(path);
        }
    }

    let mut discovered = std::collections::BTreeSet::new();
    for entry in walkdir::WalkDir::new(package_root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "dist" | "build" | ".venv" | "venv" | "vendor"
            )
        })
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if entry.file_type().is_dir() {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "skills" | "agents" | "rules" | "commands") {
                    discovered.insert(p.to_path_buf());
                }
            }
        } else if entry.file_type().is_file() {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name == "SKILL.md" {
                    if let Some(parent) = p.parent() {
                        if let Some(grandparent) = parent.parent() {
                            if grandparent.file_name().and_then(|n| n.to_str()) == Some("skills") {
                                discovered.insert(grandparent.to_path_buf());
                            } else {
                                discovered.insert(parent.to_path_buf());
                            }
                        } else {
                            discovered.insert(parent.to_path_buf());
                        }
                    }
                } else if matches!(name, "AGENTS.md" | "RULES.md") {
                    discovered.insert(p.to_path_buf());
                }
            }
        }
    }
    roots.extend(discovered);

    roots.sort();
    roots.dedup();
    let mut filtered: Vec<PathBuf> = Vec::new();
    for r in roots {
        if !filtered
            .iter()
            .any(|existing| r.starts_with(existing) && &r != existing)
        {
            filtered.push(r);
        }
    }
    filtered
}

fn read_manifest_roots(package_root: &Path) -> Option<Vec<String>> {
    for name in ["tekmor.yaml", "tekmor.yml", ".tekmor.yaml"] {
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
mod duplicate_tests {
    use super::*;

    fn write_skill(home: &Path, package: &str, body: &str) {
        let dir = packages_dir(home).join(package).join("skills/demo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn packages_with_the_same_skill_text_are_reported_as_duplicates() {
        let home = tempfile::tempdir().unwrap();
        write_skill(home.path(), "ecc", "same skill");
        write_skill(home.path(), "affaan", "same skill");
        write_skill(home.path(), "other", "different skill");
        assert_eq!(
            duplicate_packages(home.path()),
            [("affaan".to_string(), "ecc".to_string())]
        );
    }

    #[test]
    fn distinct_packages_are_not_reported() {
        let home = tempfile::tempdir().unwrap();
        write_skill(home.path(), "one", "first");
        write_skill(home.path(), "two", "second");
        assert!(duplicate_packages(home.path()).is_empty());
    }
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
    fn package_name_includes_owner_so_same_repo_names_differ() {
        let slavingia = parse_source("slavingia/skills").unwrap();
        let matt = parse_source("mattpocock/skills").unwrap();
        assert_eq!(package_name(&slavingia), "slavingia-skills");
        assert_eq!(package_name(&matt), "mattpocock-skills");
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

    #[test]
    fn discovers_nested_and_non_standard_package_roots() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/ui-ux")).unwrap();
        fs::write(
            dir.path().join(".claude/skills/ui-ux/SKILL.md"),
            "---\nname: ui-ux\n---\nUI skill",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("sub/custom/tools")).unwrap();
        fs::write(
            dir.path().join("sub/custom/tools/SKILL.md"),
            "---\nname: tool\n---\nTool skill",
        )
        .unwrap();

        let roots = discover_package_roots(dir.path());
        assert!(roots
            .iter()
            .any(|p| p.ends_with(".claude/skills") || p.ends_with("skills")));
        assert!(roots
            .iter()
            .any(|p| p.ends_with("sub/custom/tools") || p.ends_with("tools")));
    }

    #[test]
    fn path_recommend_boost_picks_best_prefix() {
        let boosts = vec![
            ("/home/pkg/skills".into(), 0.35),
            ("/home/pkg/other".into(), 0.1),
        ];
        assert!(
            (path_recommend_boost("/home/pkg/skills/tdd/SKILL.md", &boosts) - 0.35).abs() < 1e-9
        );
        assert_eq!(path_recommend_boost("/tmp/unrelated", &boosts), 0.0);
    }

    fn record(name: &str, install_path: &Path) -> PackageRecord {
        PackageRecord {
            name: name.to_string(),
            source: format!("acme/{name}"),
            git_url: format!("https://github.com/acme/{name}.git"),
            git_ref: "main".to_string(),
            install_path: install_path.display().to_string(),
            commit: Some("abc123".to_string()),
            installed_at: 1,
        }
    }

    #[test]
    fn registry_round_trips_through_packages_json() {
        let home = tempfile::tempdir().unwrap();
        let installed = record("acme-skills", &home.path().join("packages/acme-skills"));
        PackageRegistry {
            packages: vec![installed.clone()],
        }
        .save(home.path())
        .unwrap();

        let loaded = PackageRegistry::load(home.path()).unwrap();
        assert_eq!(loaded.get("acme-skills"), Some(&installed));
        assert!(loaded.get("missing").is_none());
    }

    #[test]
    fn a_missing_registry_loads_empty_and_a_corrupt_one_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        assert!(PackageRegistry::load(home.path())
            .unwrap()
            .packages
            .is_empty());
        fs::write(home.path().join("packages.json"), "not json").unwrap();
        assert!(PackageRegistry::load(home.path()).is_err());
    }

    #[test]
    fn remove_takes_the_record_out_once() {
        let home = tempfile::tempdir().unwrap();
        let installed = record("acme-skills", &home.path().join("packages/acme-skills"));
        let mut registry = PackageRegistry {
            packages: vec![installed.clone()],
        };
        assert_eq!(registry.remove("acme-skills"), Some(installed));
        assert_eq!(registry.remove("acme-skills"), None);
    }

    #[test]
    fn remove_package_deletes_its_directory_registry_entry_and_indexed_rows() {
        let home = tempfile::tempdir().unwrap();
        let config = Config::isolated(home.path().to_path_buf());
        config.ensure_dirs().unwrap();
        let install = packages_dir(home.path()).join("acme-skills");
        let skill = install.join("skills/demo/SKILL.md");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "demo skill").unwrap();
        PackageRegistry {
            packages: vec![record("acme-skills", &install)],
        }
        .save(home.path())
        .unwrap();
        let store = crate::db::store::BrainStore::open(&config.db_path).unwrap();
        store
            .upsert_indexed_item(
                crate::types::ItemType::Skill,
                "demo",
                "demo skill",
                &skill.display().to_string(),
                "package",
                Some("acme-skills"),
                &crate::db::store::content_hash("demo skill"),
                None,
            )
            .unwrap();

        let purged = remove_package(&config, "acme-skills").unwrap();

        assert_eq!(purged, 1);
        assert!(!install.exists());
        assert!(PackageRegistry::load(home.path())
            .unwrap()
            .get("acme-skills")
            .is_none());
    }

    #[test]
    fn removing_a_package_that_is_not_installed_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        let config = Config::isolated(home.path().to_path_buf());
        config.ensure_dirs().unwrap();
        let err = remove_package(&config, "ghost").unwrap_err();
        assert!(err.to_string().contains("'ghost' is not installed"));
    }

    #[test]
    fn source_inputs_resolve_to_owner_and_repo_or_fail_clearly() {
        let ssh = parse_source("git@github.com:acme/skills.git").unwrap();
        assert_eq!((ssh.owner.as_str(), ssh.repo.as_str()), ("acme", "skills"));
        let bare = parse_source("github.com/acme/skills/").unwrap();
        assert_eq!(
            (bare.owner.as_str(), bare.repo.as_str()),
            ("acme", "skills")
        );
        let http = parse_source("http://github.com/acme/skills").unwrap();
        assert_eq!(http.git_ref, "main");
        assert!(parse_source("https://github.com/acme").is_err());
        assert!(parse_source("https://github.com//skills").is_err());
        assert!(parse_source("not a source").is_err());
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn origin_repo(root: &Path) -> PathBuf {
        let origin = root.join("origin");
        fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "main"]);
        fs::write(origin.join("SKILL.md"), "first").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-q", "-m", "first"]);
        origin
    }

    fn file_url(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    #[test]
    fn clone_package_checks_out_the_branch_and_refuses_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let origin = origin_repo(root.path());
        let dest = root.path().join("installed/acme-skills");

        clone_package(&file_url(&origin), &dest, "main").unwrap();
        assert_eq!(fs::read_to_string(dest.join("SKILL.md")).unwrap(), "first");
        assert!(clone_package(&file_url(&origin), &dest, "main")
            .unwrap_err()
            .to_string()
            .contains("already exists"));
    }

    #[test]
    fn clone_package_falls_back_to_the_default_branch_when_the_ref_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let origin = origin_repo(root.path());
        let dest = root.path().join("installed/acme-skills");

        clone_package(&file_url(&origin), &dest, "no-such-branch").unwrap();
        assert_eq!(fs::read_to_string(dest.join("SKILL.md")).unwrap(), "first");
    }

    #[test]
    fn update_fast_forwards_an_install_to_the_new_commit() {
        let root = tempfile::tempdir().unwrap();
        let origin = origin_repo(root.path());
        let dest = root.path().join("installed/acme-skills");
        clone_package(&file_url(&origin), &dest, "main").unwrap();
        let first_commit = git_head(&dest).unwrap();

        fs::write(origin.join("SKILL.md"), "second").unwrap();
        git(&origin, &["commit", "-q", "-am", "second"]);
        update_package_at(&dest, "main").unwrap();

        assert_eq!(fs::read_to_string(dest.join("SKILL.md")).unwrap(), "second");
        let new_commit = git_head(&dest).unwrap();
        assert_ne!(new_commit, first_commit);
        assert_eq!(new_commit, git_head(&origin).unwrap());
    }

    #[test]
    fn update_packages_refreshes_checkouts_and_skips_bundles_and_plain_directories() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let config = Config::isolated(home.clone());
        config.ensure_dirs().unwrap();
        let origin = origin_repo(root.path());
        let checkout = packages_dir(&home).join("acme-skills");
        clone_package(&file_url(&origin), &checkout, "main").unwrap();
        let plain = packages_dir(&home).join("plain");
        fs::create_dir_all(&plain).unwrap();
        let mut bundled = record("bundled", &packages_dir(&home).join("bundled"));
        bundled.source = "tekmor/bundle:starter".into();
        let mut checkout_record = record("acme-skills", &checkout);
        checkout_record.git_ref = "main".into();
        let plain_record = record("plain", &plain);
        PackageRegistry {
            packages: vec![checkout_record, bundled, plain_record],
        }
        .save(&home)
        .unwrap();

        fs::write(origin.join("SKILL.md"), "second").unwrap();
        git(&origin, &["commit", "-q", "-am", "second"]);
        let updated = update_packages(&config, None).unwrap();

        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].name, "acme-skills");
        assert_eq!(updated[0].commit, Some(git_head(&origin).unwrap()));
        let saved = PackageRegistry::load(&home).unwrap();
        assert_eq!(saved.get("acme-skills"), Some(&updated[0]));
        assert_eq!(saved.get("bundled").unwrap().commit, Some("abc123".into()));
    }

    #[test]
    fn updating_a_package_that_is_not_installed_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        let config = Config::isolated(home.path().to_path_buf());
        config.ensure_dirs().unwrap();
        let err = update_packages(&config, Some("ghost")).unwrap_err();
        assert!(err.to_string().contains("'ghost' is not installed"));
    }

    #[test]
    fn update_refuses_a_directory_that_is_not_a_git_checkout() {
        let root = tempfile::tempdir().unwrap();
        let err = update_package_at(root.path(), "main").unwrap_err();
        assert!(err.to_string().contains("is not a git checkout"));
    }
}
