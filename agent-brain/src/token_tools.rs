//! Bounded file inspection helpers for token-efficient agent execution (Sprint D).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use walkdir::WalkDir;

use crate::tokens::estimate_tokens;

pub const DEFAULT_MAX_LINES: usize = 200;
pub const DEFAULT_MAX_BYTES: usize = 32_768;
pub const DEFAULT_GREP_MAX_MATCHES: usize = 50;
pub const MAX_GREP_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const TOKEN_TOOLS_SAVINGS_MIN_PCT: f64 = 80.0;

const BLOCKED_SEGMENTS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    "coverage",
];

#[derive(Debug, Clone, Serialize)]
pub struct TokenToolResponse {
    pub path: String,
    pub content: String,
    pub tokens_used: usize,
    pub tokens_budget: usize,
    pub truncated: bool,
    pub lines_returned: usize,
    pub total_lines: Option<usize>,
    pub file_bytes: Option<u64>,
    pub tokens_saved_vs_full_read: Option<usize>,
    pub savings_pct_vs_full_read: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GrepResponse {
    pub pattern: String,
    pub matches: Vec<GrepMatch>,
    pub match_count: usize,
    pub truncated: bool,
    pub tokens_used: usize,
    pub tokens_budget: usize,
    pub tokens_saved_vs_full_read: Option<usize>,
    pub savings_pct_vs_full_read: Option<f64>,
}

pub fn resolve_tool_path(path: &str, cwd: Option<&Path>) -> Result<PathBuf> {
    let raw = PathBuf::from(path);
    let joined = if raw.is_absolute() {
        raw
    } else if let Some(cwd) = cwd {
        cwd.join(raw)
    } else {
        std::env::current_dir().context("resolve path: no cwd")?.join(raw)
    };
    match joined.canonicalize() {
        Ok(p) => Ok(p),
        Err(_) => Ok(joined),
    }
}

pub fn is_blocked_path(path: &Path, allow_blocked: bool) -> bool {
    if allow_blocked {
        return false;
    }
    path.components().any(|c| {
        matches!(
            c,
            Component::Normal(seg) if BLOCKED_SEGMENTS.iter().any(|b| seg == *b)
        )
    })
}

pub fn file_summary(path: &Path, allow_blocked: bool, max_tokens: usize) -> Result<TokenToolResponse> {
    if is_blocked_path(path, allow_blocked) {
        bail!("path is in a blocked directory (dist/, node_modules/, etc.); set allow_blocked_paths=true with user approval");
    }
    if !path.is_file() {
        bail!("not a file: {}", path.display());
    }
    let meta = std::fs::metadata(path).context("stat file")?;
    let file_bytes = meta.len();
    let total_lines = count_lines(path)?;
    let sample = read_file_head(path, 5, DEFAULT_MAX_BYTES / 4, allow_blocked, max_tokens)?;
    let content = format!(
        "path: {}\nbytes: {}\nlines: {}\nsample (first {} lines):\n{}",
        path.display(),
        file_bytes,
        total_lines,
        sample.lines_returned,
        sample.content
    );
    let full_estimate = estimate_tokens(&std::fs::read_to_string(path).unwrap_or_default());
    build_response(
        path,
        content,
        max_tokens,
        sample.lines_returned,
        Some(total_lines),
        Some(file_bytes),
        Some(full_estimate),
    )
}

pub fn read_file_head(
    path: &Path,
    lines: usize,
    max_bytes: usize,
    allow_blocked: bool,
    max_tokens: usize,
) -> Result<TokenToolResponse> {
    if is_blocked_path(path, allow_blocked) {
        bail!("path is in a blocked directory (dist/, node_modules/, etc.); set allow_blocked_paths=true with user approval");
    }
    if !path.is_file() {
        bail!("not a file: {}", path.display());
    }
    let file_bytes = std::fs::metadata(path)?.len();
    let total_lines = count_lines(path)?;
    let max_lines = lines.clamp(1, DEFAULT_MAX_LINES);
    let mut out = String::new();
    let mut lines_returned = 0usize;
    let mut bytes_read = 0usize;
    let reader = BufReader::new(File::open(path).context("open file")?);
    for line in reader.lines() {
        let line = line.context("read line")?;
        let chunk = format!("{line}\n");
        if bytes_read + chunk.len() > max_bytes {
            break;
        }
        out.push_str(&chunk);
        bytes_read += chunk.len();
        lines_returned += 1;
        if lines_returned >= max_lines {
            break;
        }
    }
    let full_estimate = estimate_tokens(&std::fs::read_to_string(path).unwrap_or_default());
    build_response(
        path,
        out,
        max_tokens,
        lines_returned,
        Some(total_lines),
        Some(file_bytes),
        Some(full_estimate),
    )
}

pub fn read_file_tail(
    path: &Path,
    lines: usize,
    max_bytes: usize,
    allow_blocked: bool,
    max_tokens: usize,
) -> Result<TokenToolResponse> {
    if is_blocked_path(path, allow_blocked) {
        bail!("path is in a blocked directory (dist/, node_modules/, etc.); set allow_blocked_paths=true with user approval");
    }
    if !path.is_file() {
        bail!("not a file: {}", path.display());
    }
    let file_bytes = std::fs::metadata(path)?.len();
    let total_lines = count_lines(path)?;
    let max_lines = lines.clamp(1, DEFAULT_MAX_LINES);
    let ring = read_tail_lines(path, max_lines, max_bytes)?;
    let content = ring.join("\n");
    let full_estimate = estimate_tokens(&std::fs::read_to_string(path).unwrap_or_default());
    build_response(
        path,
        content,
        max_tokens,
        ring.len(),
        Some(total_lines),
        Some(file_bytes),
        Some(full_estimate),
    )
}

pub fn grep_search(
    pattern: &str,
    path: &Path,
    max_matches: usize,
    case_insensitive: bool,
    allow_blocked: bool,
    max_tokens: usize,
) -> Result<GrepResponse> {
    if pattern.trim().is_empty() {
        bail!("pattern must not be empty");
    }
    if is_blocked_path(path, allow_blocked) {
        bail!("path is in a blocked directory (dist/, node_modules/, etc.); set allow_blocked_paths=true with user approval");
    }
    let pattern_src = if case_insensitive {
        format!("(?i){}", regex::escape(pattern))
    } else {
        pattern.to_string()
    };
    let re = Regex::new(&pattern_src)
        .or_else(|_| Regex::new(&regex::escape(pattern)))
        .context("compile grep pattern")?;

    let limit = max_matches.clamp(1, 200);
    let mut matches = Vec::new();
    let mut full_bytes = 0u64;

    if path.is_file() {
        full_bytes = std::fs::metadata(path)?.len();
        grep_file(path, &re, limit, &mut matches)?;
    } else if path.is_dir() {
        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.is_dir() {
                continue;
            }
            if is_blocked_path(p, false) {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(p) {
                if meta.len() > MAX_GREP_FILE_BYTES {
                    continue;
                }
                full_bytes = full_bytes.saturating_add(meta.len());
            }
            grep_file(p, &re, limit.saturating_sub(matches.len()), &mut matches)?;
            if matches.len() >= limit {
                break;
            }
        }
    } else {
        bail!("path not found: {}", path.display());
    }

    let truncated = matches.len() >= limit;
    let mut content = String::new();
    for m in &matches {
        content.push_str(&format!("{}:{}:{}\n", m.path, m.line_number, m.line));
    }
    let tokens_used = estimate_tokens(&content).min(max_tokens);
    let full_estimate = if path.is_file() {
        estimate_tokens(&std::fs::read_to_string(path).unwrap_or_default())
    } else {
        (full_bytes / 4).max(1) as usize
    };
    let (saved, savings_pct) = savings_vs_full(tokens_used, full_estimate);

    Ok(GrepResponse {
        pattern: pattern.to_string(),
        match_count: matches.len(),
        matches,
        truncated,
        tokens_used,
        tokens_budget: max_tokens,
        tokens_saved_vs_full_read: saved,
        savings_pct_vs_full_read: savings_pct,
    })
}

fn grep_file(path: &Path, re: &Regex, limit: usize, out: &mut Vec<GrepMatch>) -> Result<()> {
    if limit == 0 {
        return Ok(());
    }
    let reader = BufReader::new(File::open(path).context("open grep file")?);
    for (idx, line) in reader.lines().enumerate() {
        let line = line.context("read grep line")?;
        if re.is_match(&line) {
            out.push(GrepMatch {
                path: path.display().to_string(),
                line_number: idx + 1,
                line,
            });
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(())
}

fn read_tail_lines(path: &Path, max_lines: usize, max_bytes: usize) -> Result<Vec<String>> {
    let mut file = File::open(path).context("open file for tail")?;
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(Vec::new());
    }
    let chunk = max_bytes.min(len as usize).max(1);
    let start = len.saturating_sub(chunk as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; chunk];
    file.read_exact(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if start > 0 {
        lines.remove(0);
    }
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    Ok(lines)
}

fn count_lines(path: &Path) -> Result<usize> {
    let reader = BufReader::new(File::open(path).context("open file for line count")?);
    Ok(reader.lines().count())
}

fn build_response(
    path: &Path,
    mut content: String,
    max_tokens: usize,
    lines_returned: usize,
    total_lines: Option<usize>,
    file_bytes: Option<u64>,
    full_read_tokens: Option<usize>,
) -> Result<TokenToolResponse> {
    let mut truncated = false;
    let mut tokens_used = estimate_tokens(&content);
    if tokens_used > max_tokens {
        truncated = true;
        let target_chars = max_tokens.saturating_mul(4);
        content.truncate(target_chars.min(content.len()));
        tokens_used = estimate_tokens(&content);
    }
    let (saved, savings_pct) = savings_vs_full(tokens_used, full_read_tokens.unwrap_or(0));

    Ok(TokenToolResponse {
        path: path.display().to_string(),
        content,
        tokens_used,
        tokens_budget: max_tokens,
        truncated,
        lines_returned,
        total_lines,
        file_bytes,
        tokens_saved_vs_full_read: saved,
        savings_pct_vs_full_read: savings_pct,
    })
}

fn savings_vs_full(used: usize, full: usize) -> (Option<usize>, Option<f64>) {
    if full == 0 || used >= full {
        return (Some(0), Some(0.0));
    }
    let saved = full - used;
    let pct = (saved as f64 / full as f64) * 100.0;
    (Some(saved), Some(pct))
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenToolsBenchScenario {
    pub name: String,
    pub tool: String,
    pub savings_pct: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenToolsBenchReport {
    pub scenarios: Vec<TokenToolsBenchScenario>,
    pub avg_savings_pct: f64,
    pub savings_min_pct: f64,
    pub passed: bool,
}

pub fn run_token_tools_bench() -> Result<TokenToolsBenchReport> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("src").join("large.log");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut body: String = (1..=2000)
        .map(|i| format!("line {i} filler text for token bench\n"))
        .collect();
    body.push_str("needle token-efficient marker here\n");
    std::fs::write(&path, &body)?;

    let full_tokens = estimate_tokens(&body);
    let mut scenarios = Vec::new();

    let summary = file_summary(&path, false, 500)?;
    scenarios.push(bench_scenario(
        "file_summary",
        "file_summary",
        summary.savings_pct_vs_full_read.unwrap_or(0.0),
    ));

    let head = read_file_head(&path, 50, DEFAULT_MAX_BYTES, false, 500)?;
    scenarios.push(bench_scenario(
        "read_file_head",
        "read_file_head",
        head.savings_pct_vs_full_read.unwrap_or(0.0),
    ));

    let tail = read_file_tail(&path, 20, DEFAULT_MAX_BYTES, false, 500)?;
    scenarios.push(bench_scenario(
        "read_file_tail",
        "read_file_tail",
        tail.savings_pct_vs_full_read.unwrap_or(0.0),
    ));

    let grep = grep_search("needle token-efficient", &path, 5, false, false, 500)?;
    let grep_savings = if full_tokens > 0 {
        (full_tokens.saturating_sub(grep.tokens_used) as f64 / full_tokens as f64) * 100.0
    } else {
        0.0
    };
    scenarios.push(bench_scenario(
        "grep_search",
        "grep_search",
        grep_savings,
    ));

    let avg_savings_pct = scenarios.iter().map(|s| s.savings_pct).sum::<f64>() / scenarios.len() as f64;
    let passed = scenarios.iter().all(|s| s.passed);

    Ok(TokenToolsBenchReport {
        scenarios,
        avg_savings_pct,
        savings_min_pct: TOKEN_TOOLS_SAVINGS_MIN_PCT,
        passed,
    })
}

fn bench_scenario(name: &str, tool: &str, savings_pct: f64) -> TokenToolsBenchScenario {
    TokenToolsBenchScenario {
        name: name.to_string(),
        tool: tool.to_string(),
        savings_pct,
        passed: savings_pct >= TOKEN_TOOLS_SAVINGS_MIN_PCT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_dist_without_override() {
        let p = Path::new("/tmp/project/dist/bundle.js");
        assert!(is_blocked_path(p, false));
        assert!(!is_blocked_path(p, true));
    }

    #[test]
    fn head_saves_tokens_on_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        let body = "x\n".repeat(5000);
        std::fs::write(&path, &body).unwrap();
        let resp = read_file_head(&path, 10, 4096, false, 500).unwrap();
        assert!(resp.savings_pct_vs_full_read.unwrap_or(0.0) >= TOKEN_TOOLS_SAVINGS_MIN_PCT);
    }

    #[test]
    fn grep_finds_needle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        std::fs::write(&path, "aaa\nneedle\nbbb\n").unwrap();
        let resp = grep_search("needle", &path, 10, false, false, 500).unwrap();
        assert_eq!(resp.matches.len(), 1);
        assert_eq!(resp.matches[0].line_number, 2);
    }

    #[test]
    fn token_tools_bench_passes() {
        let report = run_token_tools_bench().unwrap();
        assert!(report.passed, "{report:?}");
    }
}
