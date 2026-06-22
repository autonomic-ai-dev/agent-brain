//! Curated bundles shipped inside the binary (no git clone).

mod autonomic_core;
mod supervisor;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::{packages_dir, PackageRecord, PackageRegistry};
use crate::config::Config;

pub fn install_bundled(config: &Config, bundle: &str) -> Result<PackageRecord> {
    let files = match bundle {
        "autonomic-core" => autonomic_core::files(),
        "supervisor" => supervisor::files(),
        other => anyhow::bail!("unknown bundled package '{other}'"),
    };

    let install_dir = packages_dir(&config.home).join(bundle);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("remove old bundle at {}", install_dir.display()))?;
    }

    for (rel, content) in files {
        let path = install_dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }

    let now = chrono::Utc::now().timestamp();
    let record = PackageRecord {
        name: bundle.to_string(),
        source: format!("agent-brain/bundle:{bundle}"),
        git_url: String::new(),
        git_ref: env!("CARGO_PKG_VERSION").to_string(),
        install_path: install_dir.display().to_string(),
        commit: Some(env!("CARGO_PKG_VERSION").to_string()),
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
        let skill = home.join("packages/autonomic-core/.cursor/skills/find-skills/SKILL.md");
        assert!(skill.is_file(), "missing {}", skill.display());
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
        let skill = home.join("packages/supervisor/.cursor/skills/token-efficient-ops/SKILL.md");
        assert!(skill.is_file(), "missing {}", skill.display());
    }
}
