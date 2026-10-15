//! Minimal second-machine bootstrap (thin slice; full plugin marketplace is backlog).

use std::process::Command;
use std::sync::Arc;

use anyhow::{bail, Result};

use crate::config::Config;
use crate::db::store::BrainStore;
use crate::engine::Engine;
use crate::host_install::{self, HostTarget};
use crate::install;
use crate::settings::AgentBrainSettings;
use crate::sync::{self, MergePolicy, SyncSource};

/// Idempotent device bootstrap: ensure sync clone/pull, import, host install, doctor hints.
pub fn run(remote: Option<&str>, skip_install: bool) -> Result<()> {
    let config = Config::load()?;
    let settings = AgentBrainSettings::load(&config.home);
    let remote = remote
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let r = settings.sync.git.remote.clone();
            if r.is_empty() {
                None
            } else {
                Some(r)
            }
        });

    let sync_root = sync::git_sync_root(&config.home);
    if !sync_root.join(".git").is_dir() {
        let Some(remote) = remote.as_deref() else {
            bail!("sync.git.remote is unset; pass --remote URL or set it in config.yaml");
        };
        println!("Cloning memory remote…");
        sync::git_clone(&config.home, remote, &settings.sync.git.branch)?;
    } else {
        println!("Sync repo already present at {}", sync_root.display());
    }

    let engine = Arc::new(Engine::new(config.clone())?);
    if sync_root.join("bundle").join("manifest.json").is_file() {
        println!("Importing sync bundle…");
        let report = engine.import_bundle_queued(
            &sync_root.join("bundle"),
            MergePolicy::NewerWins,
            SyncSource::Git,
        )?;
        println!(
            "Imported {} (deduped {}, conflicts {}, skipped {})",
            report.imported, report.deduplicated, report.conflicts_resolved, report.skipped
        );
    } else {
        println!("No bundle/manifest.json yet — push from another device first.");
    }

    // Re-embed / refresh indexes.
    engine.bootstrap(None)?;

    if !skip_install {
        println!("Installing host integrations (--global)…");
        install::run(HostTarget::All, false, false)?;
        host_install::install_agent_brain_modes(true, true)?;
    }

    // Ensure device_id exists.
    let store = BrainStore::open(&config.db_path)?;
    let device_id = store.ensure_device_id()?;
    println!("device_id={device_id}");

    // Soft doctor: print which hosts look present.
    print_host_hints();

    println!("Bootstrap complete. Next: agent-brain doctor");
    Ok(())
}

fn print_host_hints() {
    let home = dirs::home_dir().unwrap_or_default();
    let checks = [
        ("Cursor", home.join(".cursor")),
        ("Claude", home.join(".claude")),
        ("Codex", home.join(".codex")),
        ("Gemini", home.join(".gemini")),
    ];
    println!("Detected host dirs:");
    for (name, path) in checks {
        let mark = if path.is_dir() { "yes" } else { "no" };
        println!("  {name}: {mark} ({})", path.display());
    }
    let _ = Command::new("python3").arg("--version").status();
}
