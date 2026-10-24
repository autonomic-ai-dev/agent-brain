//! Curated bundles shipped inside the binary (no git clone).

mod autonomic_core;
mod supervisor;

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

use super::{packages_dir, PackageRecord, PackageRegistry};
use crate::config::Config;
use crate::registry_sync;

pub fn install_bundled(config: &Config, bundle: &str) -> Result<PackageRecord> {
    let install_dir = packages_dir(&config.home).join(bundle);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("remove old bundle at {}", install_dir.display()))?;
    }

    let repo_dir = registry_sync::cache_dir(&config.home).join("repo");
    let candidate_paths = [
        repo_dir.join("packages").join(bundle),
        repo_dir.join(bundle),
    ];
    let mut synced_bundle = candidate_paths.iter().find(|p| p.is_dir()).cloned();

    if synced_bundle.is_none() {
        let reg_settings = crate::settings::AgentBrainSettings::load(&config.home).registry;
        if reg_settings.enabled {
            let _ = registry_sync::sync_remote(&config.home, &reg_settings);
            synced_bundle = candidate_paths.iter().find(|p| p.is_dir()).cloned();
        }
    }

    let (commit_val, source_val) = if let Some(ref synced_path) = synced_bundle {
        copy_dir_recursive(synced_path, &install_dir)?;
        let commit = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo_dir)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        (commit, format!("registry:packages/{bundle}"))
    } else {
        let files = bundled_manifest_dir(bundle)
            .ok_or_else(|| anyhow::anyhow!("package '{bundle}' not found in registry cache or bundles"))?;

        for (rel, content) in files {
            let path = install_dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
        }
        (
            Some(env!("CARGO_PKG_VERSION").to_string()),
            format!("agent-brain/bundle:{bundle}"),
        )
    };

    let now = chrono::Utc::now().timestamp();
    let record = PackageRecord {
        name: bundle.to_string(),
        source: source_val,
        git_url: String::new(),
        git_ref: "master".to_string(),
        install_path: install_dir.display().to_string(),
        commit: commit_val,
        installed_at: now,
    };

    let mut registry = PackageRegistry::load(&config.home)?;
    registry.remove(bundle);
    registry.packages.push(record.clone());
    registry.save(&config.home)?;

    if bundle == "supervisor" {
        if let Ok(store) = crate::db::store::BrainStore::open(&config.db_path) {
            let _ = crate::adoption::record_supervisor_pack(&store);
        }
    }

    Ok(record)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

pub fn bundled_manifest_dir(bundle: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match bundle {
        "autonomic-core" => Some(autonomic_core::files()),
        "supervisor" => Some(supervisor::files()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tempfile::TempDir;

    #[test]
    fn installs_autonomic_core_bundle_files() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let mut config = Config::isolated(home.clone());
        config.db_path = home.join("data").join("brain.db");
        config.data_dir = home.join("data");
        config.vectors_path = home.join("data").join("vectors.bin");
        config.ensure_dirs().unwrap();
        let record = install_bundled(&config, "autonomic-core").unwrap();
        assert_eq!(record.name, "autonomic-core");
        let skill = home.join("packages/autonomic-core/skills/find-skills/SKILL.md");
        assert!(skill.is_file(), "missing {}", skill.display());
        let rmcp = home.join("packages/autonomic-core/skills/rmcp-mcp-gateway/SKILL.md");
        assert!(rmcp.is_file(), "missing {}", rmcp.display());
    }

    #[test]
    fn installs_supervisor_bundle_files() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let mut config = Config::isolated(home.clone());
        config.db_path = home.join("data").join("brain.db");
        config.data_dir = home.join("data");
        config.vectors_path = home.join("data").join("vectors.bin");
        config.ensure_dirs().unwrap();
        let record = install_bundled(&config, "supervisor").unwrap();
        assert_eq!(record.name, "supervisor");
        let skill = home.join("packages/supervisor/skills/token-efficient-ops/SKILL.md");
        assert!(skill.is_file(), "missing {}", skill.display());
    }

    #[test]
    fn installs_agstack_bundle_files() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let mut config = Config::isolated(home.clone());
        config.db_path = home.join("data").join("brain.db");
        config.data_dir = home.join("data");
        config.vectors_path = home.join("data").join("vectors.bin");
        config.ensure_dirs().unwrap();

        let repo_agstack =
            registry_sync::cache_dir(&home).join("repo/packages/agstack/skills/agstack-mode");
        fs::create_dir_all(&repo_agstack).unwrap();
        fs::write(
            repo_agstack.join("SKILL.md"),
            "---\nname: agstack-mode\n---\n# agstack\n",
        )
        .unwrap();

        let record = install_bundled(&config, "agstack").unwrap();
        assert_eq!(record.name, "agstack");
        let skill = home.join("packages/agstack/skills/agstack-mode/SKILL.md");
        assert!(skill.is_file(), "missing {}", skill.display());
        let body = std::fs::read_to_string(&skill).unwrap();
        assert!(body.contains("agstack"));
    }
}
