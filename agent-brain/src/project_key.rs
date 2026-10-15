//! Path-independent project identity for sync and scoped memory.
//!
//! Key = SHA-256 of normalized git remote URL; else first commit OID; else `.agent-brain-id`.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::find_repo_root;

/// Resolve a stable project key for `cwd` and optionally record the local `repo_root` mapping.
pub fn resolve_project_key(cwd: &Path) -> Result<ProjectIdentity> {
    let repo_root = find_repo_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let repo_root_str = repo_root.display().to_string();

    if let Some(remote) = git_remote_url(&repo_root)? {
        let key = hash_hex(&normalize_remote_url(&remote));
        return Ok(ProjectIdentity {
            project_key: key,
            repo_root: repo_root_str,
            source: ProjectKeySource::GitRemote,
        });
    }

    if let Some(oid) = git_first_commit(&repo_root)? {
        let key = hash_hex(&oid);
        return Ok(ProjectIdentity {
            project_key: key,
            repo_root: repo_root_str,
            source: ProjectKeySource::FirstCommit,
        });
    }

    let id_path = repo_root.join(".agent-brain-id");
    let id = if id_path.is_file() {
        fs::read_to_string(&id_path)
            .context("read .agent-brain-id")?
            .trim()
            .to_string()
    } else {
        let id = Uuid::new_v4().to_string();
        fs::write(&id_path, format!("{id}\n")).context("write .agent-brain-id")?;
        id
    };
    let key = hash_hex(&id);
    Ok(ProjectIdentity {
        project_key: key,
        repo_root: repo_root_str,
        source: ProjectKeySource::AgentBrainId,
    })
}

/// True when `scope_key` looks like an absolute filesystem path (legacy v1).
pub fn looks_like_path_scope_key(scope_key: &str) -> bool {
    let p = Path::new(scope_key);
    p.is_absolute() || scope_key.starts_with('/') || scope_key.contains(":\\")
}

/// Normalize a git remote for hashing: strip credentials, lowercase host, owner/repo.
pub fn normalize_remote_url(raw: &str) -> String {
    let s = raw.trim();
    // git@host:owner/repo.git
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            let path = path.trim_end_matches(".git").trim_matches('/');
            return format!("{}/{}", host.to_ascii_lowercase(), path.to_ascii_lowercase());
        }
    }
    // https://user:pass@host/owner/repo.git
    if let Some(without_scheme) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .or_else(|| s.strip_prefix("ssh://"))
    {
        let without_creds = without_scheme
            .rsplit_once('@')
            .map(|(_, host_path)| host_path)
            .unwrap_or(without_scheme);
        let cleaned = without_creds.trim_end_matches(".git").trim_matches('/');
        return cleaned.to_ascii_lowercase();
    }
    s.trim_end_matches(".git").to_ascii_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity {
    pub project_key: String,
    pub repo_root: String,
    pub source: ProjectKeySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKeySource {
    GitRemote,
    FirstCommit,
    AgentBrainId,
}

fn hash_hex(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

fn git_remote_url(repo: &Path) -> Result<Option<String>> {
    if !repo.join(".git").exists() {
        return Ok(None);
    }
    let out = Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(repo)
        .output()
        .context("git config remote.origin.url")?;
    if !out.status.success() {
        return Ok(None);
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if url.is_empty() {
        Ok(None)
    } else {
        Ok(Some(url))
    }
}

fn git_first_commit(repo: &Path) -> Result<Option<String>> {
    if !repo.join(".git").exists() {
        return Ok(None);
    }
    let out = Command::new("git")
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .current_dir(repo)
        .output()
        .context("git rev-list first commit")?;
    if !out.status.success() {
        return Ok(None);
    }
    let oid = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if oid.is_empty() {
        Ok(None)
    } else {
        Ok(Some(oid))
    }
}

/// Ensure `.agent-brain-id` path helper for tests.
#[cfg(test)]
pub fn agent_brain_id_path(repo: &Path) -> std::path::PathBuf {
    repo.join(".agent-brain-id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_https_creds_and_git_suffix() {
        assert_eq!(
            normalize_remote_url("https://user:tok@GitHub.com/Org/Repo.git"),
            "github.com/org/repo"
        );
    }

    #[test]
    fn normalize_ssh_style() {
        assert_eq!(
            normalize_remote_url("git@github.com:Org/Repo.git"),
            "github.com/org/repo"
        );
    }

    #[test]
    fn path_scope_detection() {
        assert!(looks_like_path_scope_key("/Users/me/app"));
        assert!(!looks_like_path_scope_key("a1b2c3d4e5f6"));
    }
}
