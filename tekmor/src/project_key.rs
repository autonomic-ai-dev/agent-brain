//! Path-independent project identity for sync and scoped memory.
//!

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::config::find_repo_root;


pub fn project_key_if_known(cwd: &Path) -> Option<String> {
    let repo_root = find_repo_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    if let Ok(Some(remote)) = git_remote_url(&repo_root) {
        return Some(hash_hex(&normalize_remote_url(&remote)));
    }
    if let Ok(Some(oid)) = git_first_commit(&repo_root) {
        return Some(hash_hex(&oid));
    }
    let id = fs::read_to_string(repo_root.join(".tekmor-id")).ok()?;
    Some(hash_hex(id.trim()))
}


/// Normalize a git remote for hashing: strip credentials, lowercase host, owner/repo.
pub fn normalize_remote_url(raw: &str) -> String {
    let s = raw.trim();
    // git@host:owner/repo.git
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            let path = path.trim_end_matches(".git").trim_matches('/');
            return format!(
                "{}/{}",
                host.to_ascii_lowercase(),
                path.to_ascii_lowercase()
            );
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






#[cfg(test)]
mod tests {

    #[test]
    fn project_key_if_known_never_writes_an_id_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(project_key_if_known(dir.path()), None);
        assert!(!dir.path().join(".tekmor-id").exists());
    }

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

}
