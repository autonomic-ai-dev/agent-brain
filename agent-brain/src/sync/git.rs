//! S2 git sync — bundle lives in a dedicated repo at `~/.agent_brain/sync/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::db::store::BrainStore;
use crate::embed::Embedder;
use crate::settings::GitSyncSettings;

use super::bundle::{export_bundle, import_bundle, ImportReport, MergePolicy, SyncSource};

pub const BUNDLE_DIR_NAME: &str = "bundle";

pub fn git_sync_root(home: &Path) -> PathBuf {
    home.join("sync")
}

pub fn git_bundle_dir(home: &Path) -> PathBuf {
    git_sync_root(home).join(BUNDLE_DIR_NAME)
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GitSyncStatus {
    pub initialized: bool,
    pub remote: Option<String>,
    pub branch: String,
    pub bundle_present: bool,
    pub fact_count: Option<usize>,
}

pub fn git_status(home: &Path, settings: &GitSyncSettings) -> Result<GitSyncStatus> {
    let root = git_sync_root(home);
    let bundle = git_bundle_dir(home);
    let mut status = GitSyncStatus {
        branch: settings.branch.clone(),
        ..Default::default()
    };

    if !root.join(".git").is_dir() {
        return Ok(status);
    }
    status.initialized = true;
    status.remote = git_config_value(&root, "remote.origin.url")?;
    status.bundle_present = bundle.join("manifest.json").is_file();

    if status.bundle_present {
        let raw = fs::read_to_string(bundle.join("manifest.json"))
            .context("read bundle manifest.json")?;
        let manifest: serde_json::Value = serde_json::from_str(&raw)?;
        status.fact_count = manifest.get("fact_count").and_then(|v| v.as_u64()).map(|n| n as usize);
    }

    Ok(status)
}

pub fn init_git_repo(home: &Path, remote: Option<&str>, branch: &str) -> Result<PathBuf> {
    let root = git_sync_root(home);
    fs::create_dir_all(&root).context("create sync dir")?;

    if !root.join(".git").is_dir() {
        run_git(&root, &["init", "-b", branch])?;
    }

    write_sync_readme(&root)?;
    write_gitignore(&root)?;

    if let Some(url) = remote.filter(|s| !s.is_empty()) {
        let existing = git_config_value(&root, "remote.origin.url")?;
        if existing.as_deref() != Some(url) {
            if existing.is_some() {
                run_git(&root, &["remote", "set-url", "origin", url])?;
            } else {
                run_git(&root, &["remote", "add", "origin", url])?;
            }
        }
    }

    let bundle = git_bundle_dir(home);
    fs::create_dir_all(&bundle).context("create bundle dir")?;

    // Best-effort initial commit so the first push has a branch tip.
    let _ = run_git(&root, &["add", "README.md", ".gitignore", BUNDLE_DIR_NAME]);
    let _ = run_git(&root, &["commit", "-m", "agent-brain sync init"]);

    Ok(root)
}

/// Clone an existing sync repo (second machine setup). Refuses if `~/.agent_brain/sync` exists.
pub fn git_clone(home: &Path, remote: &str, branch: &str) -> Result<PathBuf> {
    let root = git_sync_root(home);
    if root.exists() {
        bail!(
            "sync dir already exists at {}; remove it or use sync git pull",
            root.display()
        );
    }
    fs::create_dir_all(home).context("create home dir")?;
    run_git(
        home,
        &[
            "clone",
            "--branch",
            branch,
            "--single-branch",
            remote,
            root.to_str().context("sync path not UTF-8")?,
        ],
    )?;
    Ok(root)
}

pub fn git_push(store: &BrainStore, home: &Path, settings: &GitSyncSettings) -> Result<()> {
    let root = git_sync_root(home);
    if !root.join(".git").is_dir() {
        bail!("git sync not initialized; run: agent-brain sync git init [--remote URL]");
    }
    if settings.remote.is_empty() {
        bail!("sync.git.remote is not set in config.yaml");
    }

    let remote = settings.remote.as_str();
    if git_config_value(&root, "remote.origin.url")?.is_none() {
        run_git(&root, &["remote", "add", "origin", remote])?;
    }

    let bundle = git_bundle_dir(home);
    export_bundle(store, home, Some(&bundle))?;

    run_git(&root, &["add", BUNDLE_DIR_NAME])?;
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let commit = run_git(&root, &["commit", "-m", &format!("agent-brain sync {stamp}")]);
    if let Err(err) = &commit {
        let msg = err.to_string();
        if !msg.contains("nothing to commit") {
            commit?;
        }
    }

    run_git(
        &root,
        &["push", "-u", "origin", settings.branch.as_str()],
    )?;
    Ok(())
}

pub fn git_pull(
    store: &BrainStore,
    embedder: &Embedder,
    home: &Path,
    settings: &GitSyncSettings,
) -> Result<ImportReport> {
    let root = git_sync_root(home);
    if !root.join(".git").is_dir() {
        bail!("git sync not initialized; run: agent-brain sync git init [--remote URL]");
    }

    run_git(
        &root,
        &["pull", "--ff-only", "origin", settings.branch.as_str()],
    )?;

    let bundle = git_bundle_dir(home);
    if !bundle.join("manifest.json").is_file() {
        return Ok(ImportReport::default());
    }

    import_bundle(
        store,
        embedder,
        &bundle,
        MergePolicy::NewerWins,
        SyncSource::Git,
    )
}

fn write_sync_readme(root: &Path) -> Result<()> {
    let path = root.join("README.md");
    if path.exists() {
        return Ok(());
    }
    fs::write(
        &path,
        "# agent-brain sync\n\n\
         This repo holds an exported memory bundle under `bundle/`.\n\
         Do not edit `brain.db` directly — use `agent-brain sync git push/pull`.\n",
    )?;
    Ok(())
}

fn write_gitignore(root: &Path) -> Result<()> {
    let path = root.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    fs::write(path, ".DS_Store\n")?;
    Ok(())
}

fn git_config_value(repo: &Path, key: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["config", "--get", key])
        .output()
        .context("spawn git config")?;
    if out.status.success() {
        let value = String::from_utf8(out.stdout)?.trim().to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    } else {
        Ok(None)
    }
}

fn run_git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .with_context(|| format!("spawn git {}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            if stderr.is_empty() { stdout } else { stderr }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_paths_under_home() {
        let home = PathBuf::from("/tmp/brain-home");
        assert_eq!(
            git_sync_root(&home),
            PathBuf::from("/tmp/brain-home/sync")
        );
        assert_eq!(
            git_bundle_dir(&home),
            PathBuf::from("/tmp/brain-home/sync/bundle")
        );
    }

    #[test]
    fn init_creates_git_repo_when_git_available() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        init_git_repo(home, Some("git@github.com:example/agent-brain-sync.git"), "main")
            .unwrap();
        assert!(git_sync_root(home).join(".git").is_dir());
        assert!(git_bundle_dir(home).is_dir());
        let status = git_status(home, &GitSyncSettings::default()).unwrap();
        assert!(status.initialized);
        assert_eq!(
            status.remote.as_deref(),
            Some("git@github.com:example/agent-brain-sync.git")
        );
    }
}
