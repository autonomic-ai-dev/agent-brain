use std::path::Path;
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct GitLogEntry {
    pub commit: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct GitDiffResult {
    pub diff: String,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Serialize)]
pub struct GitTagEntry {
    pub tag: String,
    pub commit: String,
    pub date: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GitCompareResult {
    pub commits: Vec<GitLogEntry>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub ahead: usize,
}

fn git_output(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(repo)
        .env_remove("GIT_EDITOR")
        .env_remove("EDITOR");
    let out = cmd
        .output()
        .map_err(|e| format!("git exec failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let hint = if stderr.contains("not a git repository") {
            " — not a git repository"
        } else if stderr.contains("ambiguous argument") {
            " — invalid ref or range"
        } else {
            ""
        };
        return Err(format!("{stderr}{hint}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git_log(repo: &Path, range: Option<&str>, max_count: usize) -> Result<Vec<GitLogEntry>, String> {
    let max_flag = format!("--max-count={}", max_count.max(1).min(500));
    let mut args = vec!["log", "--oneline", "--format=%H|%an|%ad|%s", &*max_flag];
    if let Some(r) = range {
        args.push(r);
    }
    let raw = git_output(repo, &args)?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    Ok(raw
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(GitLogEntry {
                commit: parts[0].to_string(),
                author: parts[1].to_string(),
                date: parts[2].to_string(),
                message: parts[3].to_string(),
            })
        })
        .collect())
}

pub fn git_diff(repo: &Path, range: &str, path_filter: Option<&str>) -> Result<GitDiffResult, String> {
    let stat = git_output(
        repo,
        &["diff", "--shortstat", range],
    )?;
    let (files, insertions, deletions) = parse_shortstat(&stat);

    let mut args = vec!["diff", range];
    if let Some(p) = path_filter {
        args.push("--");
        args.push(p);
    }
    let diff = git_output(repo, &args)?;

    Ok(GitDiffResult {
        diff,
        files_changed: files,
        insertions,
        deletions,
    })
}

pub fn git_tags(repo: &Path, pattern: Option<&str>, max_count: usize) -> Result<Vec<GitTagEntry>, String> {
    let fmt = "--format=%(refname:short)|%(objectname)|%(creatordate:short)|%(contents:subject)";
    let mut args = vec!["tag", "--sort=-version:refname", fmt];
    if let Some(p) = pattern {
        args.push(p);
    }
    let raw = git_output(repo, &args)?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    Ok(raw
        .lines()
        .take(max_count.max(1).min(200))
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 2 {
                return None;
            }
            Some(GitTagEntry {
                tag: parts[0].to_string(),
                commit: parts[1].to_string(),
                date: parts.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                message: parts.get(3).filter(|s| !s.is_empty()).map(|s| s.to_string()),
            })
        })
        .collect())
}

pub fn git_compare(repo: &Path, base: &str, head: &str) -> Result<GitCompareResult, String> {
    let range = format!("{base}..{head}");
    let stat = git_output(repo, &["diff", "--shortstat", &range])?;
    let (files, insertions, deletions) = parse_shortstat(&stat);

    let ahead_raw = git_output(repo, &["rev-list", "--count", &range])?;
    let ahead: usize = ahead_raw.trim().parse().unwrap_or(0);

    let log_args = [
        "log",
        "--oneline",
        "--format=%H|%an|%ad|%s",
        &range,
    ];
    let log_raw = git_output(repo, &log_args)?;
    let commits: Vec<GitLogEntry> = log_raw
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(GitLogEntry {
                commit: parts[0].to_string(),
                author: parts[1].to_string(),
                date: parts[2].to_string(),
                message: parts[3].to_string(),
            })
        })
        .collect();

    Ok(GitCompareResult {
        commits,
        files_changed: files,
        insertions,
        deletions,
        ahead,
    })
}

fn parse_shortstat(stat: &str) -> (usize, usize, usize) {
    let words: Vec<&str> = stat.split_whitespace().collect();
    let files = words
        .windows(2)
        .find(|w| w[1] == "file" || w[1] == "files")
        .and_then(|w| w[0].parse().ok())
        .unwrap_or(0);
    let insertions = words
        .windows(2)
        .find(|w| w[1].contains("insertion"))
        .and_then(|w| w[0].parse().ok())
        .unwrap_or(0);
    let deletions = words
        .windows(2)
        .find(|w| w[1].contains("deletion"))
        .and_then(|w| w[0].parse().ok())
        .unwrap_or(0);
    (files, insertions, deletions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortstat_handles_empty() {
        assert_eq!(parse_shortstat(""), (0, 0, 0));
    }

    #[test]
    fn parse_shortstat_extracts_counts() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 10 insertions(+), 2 deletions(-)"),
            (3, 10, 2)
        );
    }

    #[test]
    fn parse_shortstat_handles_only_files() {
        assert_eq!(parse_shortstat(" 1 file changed, 0 insertions(+), 0 deletions(-)"), (1, 0, 0));
    }
}
