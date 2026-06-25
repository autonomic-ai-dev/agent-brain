//! Lightweight git repo snapshot for route_task (~50–100 tokens).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotState {
    head: String,
    updated_at_ms: i64,
}

/// Capture ambient git state for the repo at `repo_root`. Returns `None` if not a git repo.
pub fn capture(repo_root: &Path, brain_home: &Path) -> Option<String> {
    capture_inner(repo_root, brain_home).ok().filter(|s| !s.is_empty())
}

fn capture_inner(repo_root: &Path, brain_home: &Path) -> Result<String> {
    if git_output(repo_root, &["rev-parse", "--is-inside-work-tree"]) != Some("true".into()) {
        return Ok(String::new());
    }

    let branch = git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|| "unknown".into());
    let head = git_output(repo_root, &["rev-parse", "HEAD"]).context("git HEAD")?;

    let dirty = git_output(repo_root, &["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);

    let ahead = git_output(repo_root, &["rev-list", "--count", "@{upstream}..HEAD"])
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n > 0);

    let state_path = state_file(brain_home, repo_root);
    let prior = read_state(&state_path);

    let mut parts = vec![format!("branch: {branch}")];
    if let Some(a) = ahead {
        parts.push(format!("ahead {a}"));
    }
    if dirty > 0 {
        parts.push(format!("dirty: {dirty} files"));
    }

    if let Some(prev) = &prior {
        if prev.head != head {
            let range = format!("{}..{}", prev.head, head);
            let commit_count = git_output(repo_root, &["rev-list", "--count", &range])
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            if commit_count > 0 {
                let date = chrono::DateTime::from_timestamp_millis(prev.updated_at_ms)
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "last visit".into());
                parts.push(format!("since {date}: +{commit_count} commits"));
                if let Some(stat) = git_output(repo_root, &["diff", "--shortstat", &range]) {
                    if let Some(files) = parse_shortstat_files(&stat) {
                        parts.push(format!("{files} files touched"));
                    }
                }
                if let Some(changed_files) = git_output(
                    repo_root,
                    &["diff", "--name-status", "--diff-filter=AMDR", &range],
                ) {
                    let lines: Vec<&str> = changed_files.lines().filter(|l| !l.is_empty()).collect();
                    if !lines.is_empty() {
                        let show: Vec<String> = lines
                            .iter()
                            .take(5)
                            .map(|l| l.to_string())
                            .collect();
                        let suffix = if lines.len() > 5 {
                            format!(" (+{} more)", lines.len() - 5)
                        } else {
                            String::new()
                        };
                        parts.push(format!("files: {}{}", show.join(", "), suffix));
                    }
                }
            }
        }
    }

    if let Some(recent) = git_output(repo_root, &["log", "-3", "--oneline"]) {
        let short: Vec<String> = recent
            .lines()
            .take(3)
            .map(|l| truncate_chars(l, 48))
            .collect();
        if !short.is_empty() {
            parts.push(format!("recent: {}", short.join("; ")));
        }
    }

    write_state(
        &state_path,
        &SnapshotState {
            head,
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        },
    )?;

    Ok(parts.join(" | "))
}

fn parse_shortstat_files(stat: &str) -> Option<u32> {
    stat.split_whitespace()
        .skip_while(|w| *w != "file" && *w != "files")
        .nth(1)
        .and_then(|n| n.parse().ok())
        .or_else(|| {
            stat.split_whitespace()
                .find(|w| w.chars().all(|c| c.is_ascii_digit()))
                .and_then(|n| n.parse().ok())
        })
}

fn truncate_chars(s: &str, max: usize) -> String {
    let n: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{n}…")
    } else {
        n
    }
}

fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(repo).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn state_file(home: &Path, repo: &Path) -> PathBuf {
    let canonical = fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let hash = format!("{:x}", Sha256::digest(canonical.display().to_string().as_bytes()));
    home.join("repo-snapshots").join(format!("{hash}.json"))
}

fn read_state(path: &Path) -> Option<SnapshotState> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_state(path: &Path, state: &SnapshotState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string(state)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortstat_extracts_file_count() {
        assert_eq!(
            parse_shortstat_files(" 3 files changed, 10 insertions(+), 2 deletions(-)"),
            Some(3)
        );
    }

    #[test]
    fn truncate_chars_limits_length() {
        assert_eq!(truncate_chars("hello world", 5), "hello…");
    }
}
