use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use walkdir::WalkDir;

use super::types::{SessionSource, SessionTranscript};

pub fn discover_sessions(config: &crate::config::Config) -> Result<Vec<SessionTranscript>> {
    let Some(home) = session_scan_home(config) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    out.extend(discover_cursor(&home, config.session_max_age_days)?);
    out.extend(discover_codex(&home, config.session_max_age_days)?);
    out.extend(discover_gemini(&home, config.session_max_age_days)?);
    out.extend(discover_opencode(&home, config.session_max_age_days)?);
    Ok(out)
}

pub fn discover_sessions_filtered(
    config: &crate::config::Config,
    sources: &[SessionSource],
) -> Result<Vec<SessionTranscript>> {
    let all = discover_sessions(config)?;
    if sources.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|s| sources.contains(&s.source))
        .collect())
}

fn discover_cursor(home: &Path, max_age_days: u64) -> Result<Vec<SessionTranscript>> {
    let mut out = Vec::new();
    let pattern = format!(
        "{}/.cursor/projects/**/agent-transcripts/**/*.jsonl",
        home.display()
    );
    if let Ok(entries) = glob::glob(&pattern) {
        for path in entries.flatten() {
            if !is_recent_enough(&path, max_age_days) {
                continue;
            }
            let session_id = session_id_from_jsonl_path(SessionSource::Cursor, &path);
            out.push(SessionTranscript::jsonl(
                path,
                SessionSource::Cursor,
                session_id,
            ));
        }
    }
    Ok(out)
}

fn discover_codex(home: &Path, max_age_days: u64) -> Result<Vec<SessionTranscript>> {
    let mut out = Vec::new();
    let root = home.join(".codex/sessions");
    if !root.is_dir() {
        return Ok(out);
    }
    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if !is_recent_enough(&path, max_age_days) {
            continue;
        }
        let session_id = session_id_from_jsonl_path(SessionSource::Codex, &path);
        out.push(SessionTranscript::jsonl(
            path,
            SessionSource::Codex,
            session_id,
        ));
    }
    Ok(out)
}

fn discover_gemini(home: &Path, max_age_days: u64) -> Result<Vec<SessionTranscript>> {
    let mut out = Vec::new();
    let root = home.join(".gemini");
    if !root.is_dir() {
        return Ok(out);
    }
    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path().to_path_buf();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name != "transcript.jsonl" && name != "transcript_full.jsonl" {
            continue;
        }
        // Prefer compact transcript; skip _full when compact exists in same dir.
        if name == "transcript_full.jsonl" {
            let compact = path.with_file_name("transcript.jsonl");
            if compact.is_file() {
                continue;
            }
        }
        if !is_recent_enough(&path, max_age_days) {
            continue;
        }
        let session_id = session_id_from_jsonl_path(SessionSource::Gemini, &path);
        out.push(SessionTranscript::jsonl(
            path,
            SessionSource::Gemini,
            session_id,
        ));
    }
    Ok(out)
}

pub fn session_scan_home(config: &crate::config::Config) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TEKMOR_SESSION_HOME") {
        return Some(PathBuf::from(p));
    }
    dirs::home_dir().or_else(|| Some(config.home.clone()))
}

fn discover_opencode(home: &Path, max_age_days: u64) -> Result<Vec<SessionTranscript>> {
    let db_path = opencode_db_path(home);
    if !db_path.is_file() {
        return Ok(Vec::new());
    }
    super::opencode::discover_sessions(&db_path, max_age_days)
}

pub fn opencode_db_path(home: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("TEKMOR_OPENCODE_DB") {
        return PathBuf::from(p);
    }
    home.join(".local/share/opencode/opencode.db")
}

fn session_id_from_jsonl_path(source: SessionSource, path: &Path) -> String {
    if source == SessionSource::Gemini {
        if let Some(id) = gemini_brain_uuid(path) {
            return id;
        }
    }
    if source == SessionSource::Cursor {
        if let Some(id) = cursor_transcript_uuid(path) {
            return id;
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn gemini_brain_uuid(path: &Path) -> Option<String> {
    let parts: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    for (i, part) in parts.iter().enumerate() {
        if part == "brain" {
            if let Some(uuid) = parts.get(i + 1) {
                if !uuid.is_empty() {
                    return Some(uuid.to_string());
                }
            }
        }
    }
    None
}

fn cursor_transcript_uuid(path: &Path) -> Option<String> {
    let parts: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    for (i, part) in parts.iter().enumerate() {
        if part == "agent-transcripts" {
            if let Some(uuid) = parts.get(i + 1) {
                if !uuid.is_empty() {
                    return Some(uuid.to_string());
                }
            }
        }
    }
    path.file_stem().and_then(|s| s.to_str()).map(String::from)
}

fn is_recent_enough(path: &Path, max_age_days: u64) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return true;
    };
    age.as_secs() <= max_age_days * 24 * 3600
}

pub fn count_by_source(
    sessions: &[SessionTranscript],
) -> std::collections::HashMap<SessionSource, usize> {
    let mut counts = std::collections::HashMap::new();
    for s in sessions {
        *counts.entry(s.source).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_uuid_from_brain_path() {
        let path = PathBuf::from(
            "/home/u/.gemini/antigravity-cli/brain/c1db6ad4-223d-4e9b-b281-c505a68c6eee/.system_generated/logs/transcript.jsonl",
        );
        assert_eq!(
            gemini_brain_uuid(&path).as_deref(),
            Some("c1db6ad4-223d-4e9b-b281-c505a68c6eee")
        );
    }

    #[test]
    fn cursor_uuid_from_transcript_path() {
        let path =
            PathBuf::from("/home/u/.cursor/projects/foo/agent-transcripts/abc-123/abc-123.jsonl");
        assert_eq!(cursor_transcript_uuid(&path).as_deref(), Some("abc-123"));
    }
}
