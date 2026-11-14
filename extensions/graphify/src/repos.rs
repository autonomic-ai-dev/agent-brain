use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::GraphifyRepoRecord;

const REPOS_FILE: &str = "graphify/repos.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReposRegistry {
    pub repos: Vec<GraphifyRepoRecord>,
}

impl ReposRegistry {
    pub fn load(home: &Path) -> Result<Self> {
        let path = home.join(REPOS_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).context("parse graphify/repos.json")
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let path = home.join(REPOS_FILE);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let pretty = serde_json::to_string_pretty(self)?;
        fs::write(path, format!("{pretty}\n")).context("write graphify/repos.json")
    }

    pub fn find(&self, repo_root: &str) -> Option<&GraphifyRepoRecord> {
        self.repos.iter().find(|r| r.repo_root == repo_root)
    }

    pub fn find_mut(&mut self, repo_root: &str) -> Option<&mut GraphifyRepoRecord> {
        self.repos.iter_mut().find(|r| r.repo_root == repo_root)
    }
}

fn canonical_repo(repo: &Path) -> Result<PathBuf> {
    let abs = if repo.is_absolute() {
        repo.to_path_buf()
    } else {
        std::env::current_dir()?.join(repo)
    };
    fs::canonicalize(&abs).with_context(|| format!("resolve repo path {}", abs.display()))
}

pub fn enable_repo(home: &Path, repo: &Path, graphify_bin: &str) -> Result<GraphifyRepoRecord> {
    let repo_root = canonical_repo(repo)?;
    let repo_str = repo_root.display().to_string();
    let mut reg = ReposRegistry::load(home)?;
    let now = chrono::Utc::now().timestamp();
    if let Some(existing) = reg.find_mut(&repo_str) {
        existing.enabled_at = now;
    } else {
        reg.repos.push(GraphifyRepoRecord {
            repo_root: repo_str.clone(),
            enabled_at: now,
            last_ingest_at: None,
            last_graph_mtime: None,
        });
    }
    reg.save(home)?;
    install_graphify_hook(&repo_root, graphify_bin)?;
    append_ingest_hook(&repo_root)?;
    Ok(reg.find(&repo_str).cloned().unwrap())
}

pub fn disable_repo(home: &Path, repo: &Path) -> Result<()> {
    let repo_root = canonical_repo(repo)?;
    let repo_str = repo_root.display().to_string();
    let mut reg = ReposRegistry::load(home)?;
    reg.repos.retain(|r| r.repo_root != repo_str);
    reg.save(home)
}

pub fn list_repos(home: &Path) -> Result<Vec<GraphifyRepoRecord>> {
    Ok(ReposRegistry::load(home)?.repos)
}

pub fn repo_status(home: &Path, repo: Option<&Path>) -> Result<Vec<GraphifyRepoRecord>> {
    let reg = ReposRegistry::load(home)?;
    if let Some(repo) = repo {
        let repo_root = canonical_repo(repo)?;
        let repo_str = repo_root.display().to_string();
        Ok(reg
            .repos
            .into_iter()
            .filter(|r| r.repo_root == repo_str)
            .collect())
    } else {
        Ok(reg.repos)
    }
}

pub fn touch_ingest(home: &Path, repo_root: &Path, graph_mtime: Option<i64>) -> Result<()> {
    let repo_str = repo_root.display().to_string();
    let mut reg = ReposRegistry::load(home)?;
    let now = chrono::Utc::now().timestamp();
    let entry = reg.find_mut(&repo_str).with_context(|| {
        format!(
            "repo {} is not graphify-enabled; run: tekmor graphify enable --repo {}",
            repo_str, repo_str
        )
    })?;
    entry.last_ingest_at = Some(now);
    if let Some(m) = graph_mtime {
        entry.last_graph_mtime = Some(m);
    }
    reg.save(home)
}

fn install_graphify_hook(repo_root: &Path, graphify_bin: &str) -> Result<()> {
    let status = std::process::Command::new(graphify_bin)
        .args(["hook", "install"])
        .current_dir(repo_root)
        .status();
    let Ok(status) = status else {
        eprintln!(
            "Warning: `{graphify_bin}` not found on PATH — enable saved, but git hook not installed. \
             Install graphify and run: graphify hook install"
        );
        return Ok(());
    };
    if !status.success() {
        bail!("graphify hook install failed with status {status}");
    }
    Ok(())
}

fn append_ingest_hook(repo_root: &Path) -> Result<()> {
    let hook_path = repo_root.join(".git").join("hooks").join("post-commit");
    if !hook_path.exists() {
        return Ok(());
    }
    let marker = "tekmor graphify ingest";
    let raw = fs::read_to_string(&hook_path)?;
    if raw.contains(marker) {
        return Ok(());
    }
    let line = r#"
# tekmor graphify ingest (appended by `tekmor graphify enable`)
if command -v tekmor >/dev/null 2>&1; then
  tekmor graphify ingest --repo "$(git rev-parse --show-toplevel)" --trigger git_hook >/dev/null 2>&1 || true
fi
"#;
    fs::write(&hook_path, format!("{raw}{line}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }
    Ok(())
}
