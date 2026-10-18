//! Bounded file inspection helpers for token-efficient agent execution (Sprint D).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use walkdir::WalkDir;

use crate::tokens::estimate_tokens;
use crate::types::{MustApply, SuggestedNativeTool};

pub const NATIVE_TOOL_SUGGEST_LIMIT: usize = 4;

pub const DEFAULT_MAX_LINES: usize = 200;
pub const DEFAULT_MAX_BYTES: usize = 32_768;
pub const DEFAULT_GREP_MAX_MATCHES: usize = 50;
pub const MAX_GREP_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Lines longer than this are skipped (e.g. Codex session JSONL blobs) instead of failing grep.
pub const MAX_GREP_LINE_BYTES: usize = 256 * 1024;
pub const MAX_GREP_MATCH_CHARS: usize = 512;
pub const TOKEN_TOOLS_SAVINGS_MIN_PCT: f64 = 80.0;

pub const BLOCKED_SEGMENTS: &[&str] = &[
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
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lines_skipped_oversized: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListDirectoryResponse {
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
    pub entry_count: usize,
    pub total_entries: usize,
    pub truncated: bool,
    pub tokens_used: usize,
    pub tokens_budget: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileMatch {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindFilesResponse {
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    pub matches: Vec<FileMatch>,
    pub match_count: usize,
    pub truncated: bool,
    pub tokens_used: usize,
    pub tokens_budget: usize,
}

pub fn resolve_tool_path(path: &str, cwd: Option<&Path>) -> Result<PathBuf> {
    let raw = PathBuf::from(path);
    let joined = if raw.is_absolute() {
        raw
    } else if let Some(cwd) = cwd {
        cwd.join(raw)
    } else {
        std::env::current_dir()
            .context("resolve path: no cwd")?
            .join(raw)
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

pub fn file_summary(
    path: &Path,
    allow_blocked: bool,
    max_tokens: usize,
) -> Result<TokenToolResponse> {
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
    let mut reader = BufReader::new(File::open(path).context("open file")?);
    loop {
        if lines_returned >= max_lines {
            break;
        }
        match read_bounded_line(&mut reader, max_bytes.min(MAX_GREP_LINE_BYTES)) {
            Ok(None) => break,
            Ok(Some(line)) => {
                let chunk = format!("{line}\n");
                if bytes_read + chunk.len() > max_bytes {
                    break;
                }
                out.push_str(&chunk);
                bytes_read += chunk.len();
                lines_returned += 1;
            }
            Err(BoundedLineError::Oversized) => continue,
            Err(BoundedLineError::Io(e)) => return Err(e).context("read line"),
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

pub fn read_file(
    path: &Path,
    start_line: usize,
    end_line: usize,
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
    let start = start_line.max(1);
    let end = end_line.max(start).min(start + DEFAULT_MAX_LINES);
    let mut out = String::new();
    let mut lines_returned = 0usize;
    let mut bytes_read = 0usize;
    let mut reader = BufReader::new(File::open(path).context("open file")?);
    let mut current_line = 1usize;
    loop {
        if current_line > end {
            break;
        }
        match read_bounded_line(&mut reader, max_bytes.min(MAX_GREP_LINE_BYTES)) {
            Ok(None) => break,
            Ok(Some(line)) => {
                if current_line >= start {
                    let chunk = format!("{line}\n");
                    if bytes_read + chunk.len() > max_bytes {
                        break;
                    }
                    out.push_str(&chunk);
                    bytes_read += chunk.len();
                    lines_returned += 1;
                }
                current_line += 1;
            }
            Err(BoundedLineError::Oversized) => {
                current_line += 1;
                continue;
            }
            Err(BoundedLineError::Io(e)) => return Err(e).context("read line"),
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

pub fn list_directory(
    path: &Path,
    max_entries: usize,
    include_hidden: bool,
    allow_blocked: bool,
    max_tokens: usize,
) -> Result<ListDirectoryResponse> {
    if is_blocked_path(path, allow_blocked) {
        bail!("path is in a blocked directory (dist/, node_modules/, etc.); set allow_blocked_paths=true with user approval");
    }
    if !path.is_dir() {
        bail!("not a directory: {}", path.display());
    }

    let mut raw_entries = Vec::new();
    for entry in std::fs::read_dir(path).context("read directory")? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !include_hidden && file_name.starts_with('.') {
            continue;
        }
        let full_path = entry.path();
        if is_blocked_path(&full_path, allow_blocked) {
            continue;
        }
        let file_type = entry.file_type().ok();
        let metadata = entry.metadata().ok();
        let size_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_ms = metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);

        raw_entries.push(DirectoryEntry {
            name: file_name,
            path: full_path.to_string_lossy().to_string(),
            is_dir: file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false),
            is_file: file_type.as_ref().map(|t| t.is_file()).unwrap_or(false),
            is_symlink: file_type.as_ref().map(|t| t.is_symlink()).unwrap_or(false),
            size_bytes,
            modified_ms,
        });
    }

    // Sort: directories first, then alphabetical by name
    raw_entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    let total_entries = raw_entries.len();
    let limit = max_entries.clamp(1, 500);
    let mut selected = Vec::new();
    let mut truncated = false;

    for entry in raw_entries {
        if selected.len() >= limit {
            truncated = true;
            break;
        }
        selected.push(entry);
        let current_json = serde_json::to_string(&selected).unwrap_or_default();
        let tokens = estimate_tokens(&current_json);
        if tokens > max_tokens && selected.len() > 1 {
            selected.pop();
            truncated = true;
            break;
        }
    }

    let tokens_used = estimate_tokens(&serde_json::to_string(&selected).unwrap_or_default());
    Ok(ListDirectoryResponse {
        path: path.to_string_lossy().to_string(),
        entry_count: selected.len(),
        entries: selected,
        total_entries,
        truncated,
        tokens_used,
        tokens_budget: max_tokens,
    })
}

pub fn find_files(
    root: &Path,
    pattern: Option<&str>,
    extension: Option<&str>,
    max_depth: usize,
    max_matches: usize,
    allow_blocked: bool,
    max_tokens: usize,
) -> Result<FindFilesResponse> {
    if is_blocked_path(root, allow_blocked) {
        bail!(
            "root path is in a blocked directory; set allow_blocked_paths=true with user approval"
        );
    }
    if !root.is_dir() {
        bail!("not a directory: {}", root.display());
    }

    let pattern_lower = pattern.map(|p| p.to_lowercase());
    let ext_lower = extension.map(|e| e.trim_start_matches('.').to_lowercase());
    let limit = max_matches.clamp(1, 500);

    let walker = WalkDir::new(root)
        .max_depth(max_depth.clamp(1, 30))
        .follow_links(false)
        .into_iter();

    let mut matches = Vec::new();
    let mut truncated = false;

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path == root {
            continue;
        }

        if is_blocked_path(path, allow_blocked) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(ref ext) = ext_lower {
            if path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase())
                != Some(ext.clone())
            {
                continue;
            }
        }

        if let Some(ref pat) = pattern_lower {
            let path_str = path.to_string_lossy().to_lowercase();
            if !name.to_lowercase().contains(pat) && !path_str.contains(pat) {
                continue;
            }
        }

        let file_type = entry.file_type();
        let is_dir = file_type.is_dir();
        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

        matches.push(FileMatch {
            path: path.to_string_lossy().to_string(),
            name,
            is_dir,
            size_bytes,
        });

        if matches.len() >= limit {
            truncated = true;
            break;
        }

        let current_json = serde_json::to_string(&matches).unwrap_or_default();
        if estimate_tokens(&current_json) > max_tokens && matches.len() > 1 {
            matches.pop();
            truncated = true;
            break;
        }
    }

    let tokens_used = estimate_tokens(&serde_json::to_string(&matches).unwrap_or_default());
    Ok(FindFilesResponse {
        root: root.to_string_lossy().to_string(),
        pattern: pattern.map(String::from),
        extension: extension.map(String::from),
        match_count: matches.len(),
        matches,
        truncated,
        tokens_used,
        tokens_budget: max_tokens,
    })
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
    let mut lines_skipped_oversized = 0usize;

    if path.is_file() {
        let meta = std::fs::metadata(path)?;
        full_bytes = meta.len();
        if full_bytes > MAX_GREP_FILE_BYTES {
            bail!(
                "file too large for grep ({} bytes > {}); narrow path or use read_file_tail on a smaller slice",
                full_bytes,
                MAX_GREP_FILE_BYTES
            );
        }
        let skipped = grep_file(path, &re, limit, &mut matches)?;
        lines_skipped_oversized = skipped;
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
            lines_skipped_oversized +=
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
        lines_skipped_oversized,
    })
}

fn grep_file(path: &Path, re: &Regex, limit: usize, out: &mut Vec<GrepMatch>) -> Result<usize> {
    if limit == 0 {
        return Ok(0);
    }
    let mut reader = BufReader::new(File::open(path).context("open grep file")?);
    let mut line_no = 0usize;
    let mut skipped_oversized = 0usize;
    loop {
        line_no += 1;
        match read_bounded_line(&mut reader, MAX_GREP_LINE_BYTES) {
            Ok(None) => break,
            Ok(Some(line)) => {
                if re.is_match(&line) {
                    out.push(GrepMatch {
                        path: path.display().to_string(),
                        line_number: line_no,
                        line: truncate_for_match(&line, MAX_GREP_MATCH_CHARS),
                    });
                    if out.len() >= limit {
                        break;
                    }
                }
            }
            Err(BoundedLineError::Oversized) => {
                skipped_oversized += 1;
            }
            Err(BoundedLineError::Io(e)) => return Err(e).context("read grep line"),
        }
    }
    Ok(skipped_oversized)
}

enum BoundedLineError {
    Oversized,
    Io(std::io::Error),
}

/// Read one line up to `max_bytes`; skip oversized lines without failing the whole search.
fn read_bounded_line(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> Result<Option<String>, BoundedLineError> {
    let mut buf = Vec::new();
    loop {
        let chunk = {
            let available = match reader.fill_buf() {
                Ok([]) => break,
                Ok(b) => b,
                Err(e) => return Err(BoundedLineError::Io(e)),
            };
            available.to_vec()
        };

        if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
            let oversize = buf.len() + pos > max_bytes;
            let consume = pos + 1;
            if !oversize {
                buf.extend_from_slice(&chunk[..pos]);
            }
            reader.consume(consume);
            return if oversize {
                Err(BoundedLineError::Oversized)
            } else {
                Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
            };
        }

        let chunk_len = chunk.len();
        if buf.len() + chunk_len > max_bytes {
            reader.consume(chunk_len);
            skip_until_newline(reader)?;
            return Err(BoundedLineError::Oversized);
        }
        buf.extend_from_slice(&chunk);
        reader.consume(chunk_len);
    }

    if buf.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }
}

fn skip_until_newline(reader: &mut impl BufRead) -> Result<(), BoundedLineError> {
    loop {
        let (consume, done) = {
            let available = match reader.fill_buf() {
                Ok([]) => return Ok(()),
                Ok(b) => b,
                Err(e) => return Err(BoundedLineError::Io(e)),
            };
            match available.iter().position(|&b| b == b'\n') {
                Some(pos) => (pos + 1, true),
                None => (available.len(), false),
            }
        };
        reader.consume(consume);
        if done {
            return Ok(());
        }
    }
}

fn truncate_for_match(line: &str, max_chars: usize) -> String {
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    let truncated: String = line.chars().take(max_chars).collect();
    format!("{truncated}…")
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
    let mut reader = BufReader::new(File::open(path).context("open file for line count")?);
    let mut count = 0usize;
    loop {
        match read_bounded_line(&mut reader, MAX_GREP_LINE_BYTES) {
            Ok(None) => break,
            Ok(Some(_)) | Err(BoundedLineError::Oversized) => count += 1,
            Err(BoundedLineError::Io(e)) => return Err(e).context("count lines"),
        }
    }
    Ok(count)
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

/// Rank agent-brain bounded-read tools for file/log exploration queries.
pub fn suggest_native_token_tools(
    user_message: &str,
    open_files: &[String],
    phase: &str,
    must_apply: &[MustApply],
) -> Vec<SuggestedNativeTool> {
    let mut candidates: Vec<SuggestedNativeTool> = Vec::new();
    let lower = user_message.to_lowercase();
    let supervisor = crate::retrieval::supervisor_query_strength(user_message);
    let file_intent = file_exploration_intent(&lower, open_files);
    let verify_intent = verify_command_intent(&lower);

    if !file_intent && supervisor < 0.15 && phase != "debugging" && !verify_intent {
        return candidates;
    }

    let constraint_boost = if must_apply.is_empty() { 0.0 } else { 0.12 };

    if verify_intent {
        candidates.push(SuggestedNativeTool {
            tool: "agent-brain verify".into(),
            description: "Run verification commands with exit code capture and learning".into(),
            rationale: "Capture real exits to build reliable recipes and quality signals".into(),
            score: 0.90 + constraint_boost + supervisor * 0.05,
        });
    }

    if grep_intent(&lower) || file_intent {
        candidates.push(SuggestedNativeTool {
            tool: "grep_search".into(),
            description: "Search file or directory for a pattern without full reads".into(),
            rationale: "Find strings before loading file contents".into(),
            score: 0.88 + constraint_boost + supervisor * 0.1,
        });
    }

    if file_intent || open_files.is_empty() {
        candidates.push(SuggestedNativeTool {
            tool: "file_summary".into(),
            description: "Bytes, line count, and 5-line sample".into(),
            rationale: "Check file size before reading".into(),
            score: 0.82 + supervisor * 0.08,
        });
    }

    if log_intent(&lower) || file_intent {
        candidates.push(SuggestedNativeTool {
            tool: "read_file_head".into(),
            description: "First N lines (default 200, byte-capped)".into(),
            rationale: "Bounded read for large or unknown files".into(),
            score: 0.78 + constraint_boost,
        });
        if log_intent(&lower) {
            candidates.push(SuggestedNativeTool {
                tool: "read_file_tail".into(),
                description: "Last N lines — useful for logs".into(),
                rationale: "Recent log lines without full file load".into(),
                score: 0.8 + constraint_boost,
            });
        }
    }

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.dedup_by(|a, b| a.tool == b.tool);
    candidates.truncate(NATIVE_TOOL_SUGGEST_LIMIT);
    candidates
}

pub fn anti_pattern_topic_for_path(path: &str) -> String {
    let lower = path.to_lowercase();
    for segment in BLOCKED_SEGMENTS {
        if lower.contains(segment) {
            return format!("no-read-{segment}");
        }
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    format!("no-full-read-{name}")
}

pub fn anti_pattern_fact_for_path(path: &str) -> String {
    format!("Never read {path} whole — use grep_search, file_summary, read_file_head/tail.")
}

fn file_exploration_intent(lower: &str, open_files: &[String]) -> bool {
    if !open_files.is_empty() {
        return true;
    }
    [
        "read", "file", "log", "grep", "cat", "find", "search", "debug", "error", "stack", "trace",
        "dist", "artifact", "build",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn grep_intent(lower: &str) -> bool {
    [
        "grep", "find", "search", "where", "locate", "pattern", "string",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn log_intent(lower: &str) -> bool {
    ["log", "trace", "stderr", "tail", "error", "crash", "panic"]
        .iter()
        .any(|k| lower.contains(k))
}

fn verify_command_intent(lower: &str) -> bool {
    [
        "verify",
        "test",
        "check",
        "clippy",
        "lint",
        "proof",
        "benchmark",
    ]
    .iter()
    .any(|k| lower.contains(k))
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
    scenarios.push(bench_scenario("grep_search", "grep_search", grep_savings));

    let avg_savings_pct =
        scenarios.iter().map(|s| s.savings_pct).sum::<f64>() / scenarios.len() as f64;
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
    fn grep_skips_oversized_jsonl_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let huge = "x".repeat(MAX_GREP_LINE_BYTES + 1024);
        std::fs::write(
            &path,
            format!("{{\"small\":\"needle\"}}\n{huge}\n{{\"tail\":\"needle\"}}\n"),
        )
        .unwrap();
        let resp = grep_search("needle", &path, 10, false, false, 500).unwrap();
        assert_eq!(resp.matches.len(), 2);
        assert_eq!(resp.lines_skipped_oversized, 1);
    }

    #[test]
    fn grep_handles_binary_utf8_lossy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mixed.jsonl");
        let mut bytes = b"normal needle line\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe, b'n', 0x00]);
        bytes.extend_from_slice(b"\nneedle after binary\n");
        std::fs::write(&path, bytes).unwrap();
        let resp = grep_search("needle", &path, 10, false, false, 500).unwrap();
        assert!(resp.matches.len() >= 2);
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
    fn test_read_file_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("numbered.txt");
        let content = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, content).unwrap();

        let resp = read_file(&path, 5, 8, 4096, false, 500).unwrap();
        assert_eq!(resp.lines_returned, 4);
        assert!(resp.content.contains("line 5"));
        assert!(resp.content.contains("line 8"));
        assert!(!resp.content.contains("line 4"));
        assert!(!resp.content.contains("line 9"));
    }

    #[test]
    fn test_list_directory_returns_sorted_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub_dir")).unwrap();
        std::fs::write(dir.path().join("file_b.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("file_a.txt"), "world").unwrap();
        std::fs::write(dir.path().join(".hidden"), "secret").unwrap();

        let resp = list_directory(dir.path(), 50, false, false, 500).unwrap();
        assert_eq!(resp.total_entries, 3); // 1 dir + 2 non-hidden files
        assert_eq!(resp.entries[0].name, "sub_dir");
        assert!(resp.entries[0].is_dir);
        assert_eq!(resp.entries[1].name, "file_a.txt");
        assert_eq!(resp.entries[2].name, "file_b.txt");

        // Hidden entries included when requested
        let resp_hidden = list_directory(dir.path(), 50, true, false, 500).unwrap();
        assert_eq!(resp_hidden.total_entries, 4);
    }

    #[test]
    fn test_find_files_pattern_and_extension() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(sub.join("lib.rs"), "pub mod foo;").unwrap();
        std::fs::write(sub.join("readme.md"), "# Hello").unwrap();

        // Filter by extension
        let resp_ext = find_files(dir.path(), None, Some("rs"), 5, 50, false, 500).unwrap();
        assert_eq!(resp_ext.match_count, 2);
        assert!(resp_ext.matches.iter().any(|m| m.name == "main.rs"));
        assert!(resp_ext.matches.iter().any(|m| m.name == "lib.rs"));

        // Filter by pattern
        let resp_pat = find_files(dir.path(), Some("main"), None, 5, 50, false, 500).unwrap();
        assert_eq!(resp_pat.match_count, 1);
        assert_eq!(resp_pat.matches[0].name, "main.rs");
    }

    #[test]
    fn token_tools_bench_passes() {
        let report = run_token_tools_bench().unwrap();
        assert!(report.passed, "{report:?}");
    }
}
