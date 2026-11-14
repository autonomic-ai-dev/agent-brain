use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

use super::parse::parse_user_line;
use super::types::{SessionSource, SessionTranscript};

const MAX_SESSIONS: usize = 150;

pub fn discover_sessions(db_path: &Path, max_age_days: u64) -> Result<Vec<SessionTranscript>> {
    let conn = open_readonly(db_path)?;
    let cutoff_ms = session_cutoff_ms(max_age_days);
    let mut stmt = conn
        .prepare(
            "SELECT id, time_updated FROM session
             WHERE time_updated >= ?1
             ORDER BY time_updated DESC
             LIMIT ?2",
        )
        .context("prepare session list")?;
    let rows = stmt.query_map(rusqlite::params![cutoff_ms, MAX_SESSIONS as i64], |row| {
        let id: String = row.get(0)?;
        Ok(id)
    })?;

    let mut out = Vec::new();
    for id in rows.flatten() {
        out.push(SessionTranscript::opencode(db_path.to_path_buf(), id));
    }
    Ok(out)
}

pub fn extract_user_messages(db_path: &Path, session_id: &str, max: usize) -> Result<Vec<String>> {
    let conn = open_readonly(db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT p.data FROM message m
             JOIN part p ON p.message_id = m.id
             WHERE m.session_id = ?1
               AND json_extract(m.data, '$.role') = 'user'
               AND json_extract(p.data, '$.type') = 'text'
             ORDER BY m.time_created ASC
             LIMIT ?2",
        )
        .context("prepare opencode user messages")?;

    let rows = stmt.query_map(rusqlite::params![session_id, max as i64], |row| {
        let data: String = row.get(0)?;
        Ok(data)
    })?;

    let mut out = Vec::new();
    for data in rows.flatten() {
        if let Some(text) = parse_opencode_part(&data) {
            if text.len() >= 20 {
                out.push(text);
            }
        }
    }
    Ok(out)
}

fn parse_opencode_part(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("text") {
        return None;
    }
    let text = v.get("text").and_then(|t| t.as_str())?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn open_readonly(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open opencode db {}", path.display()))
}

fn session_cutoff_ms(max_age_days: u64) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    now - (max_age_days as i64 * 24 * 3600 * 1000)
}

/// Load user messages for any transcript kind.
pub fn load_user_messages(session: &SessionTranscript, max: usize) -> Result<Vec<String>> {
    if let Some(path) = &session.jsonl_path {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read session file {}", path.display()))?;
        return Ok(extract_from_jsonl(session.source, &raw, max));
    }
    if let (Some(db), SessionSource::OpenCode) = (&session.opencode_db, session.source) {
        return extract_user_messages(db, &session.session_id, max);
    }
    Ok(Vec::new())
}

fn extract_from_jsonl(source: SessionSource, raw: &str, max: usize) -> Vec<String> {
    use std::io::{BufRead, BufReader};

    let mut out = Vec::new();
    for line in BufReader::new(raw.as_bytes()).lines().map_while(Result::ok) {
        if out.len() >= max {
            break;
        }
        if let Some(text) = parse_user_line(source, &line) {
            if text.len() >= 20 {
                out.push(text);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn seed_db(dir: &TempDir) -> std::path::PathBuf {
        let path = dir.path().join("opencode.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (id TEXT PRIMARY KEY, time_updated INTEGER NOT NULL);
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            INSERT INTO session VALUES ('ses_test', 9999999999999);
            INSERT INTO message VALUES ('msg1', 'ses_test', 1, '{"role":"user"}');
            INSERT INTO part VALUES ('p1', 'msg1', 'ses_test', 1, '{"type":"text","text":"analyze teams-cli repo for opensource growth"}');
            "#,
        )
        .unwrap();
        path
    }

    #[test]
    fn discovers_and_reads_opencode_session() {
        let dir = TempDir::new().unwrap();
        let db = seed_db(&dir);
        let sessions = discover_sessions(&db, 90).unwrap();
        assert_eq!(sessions.len(), 1);
        let msgs = extract_user_messages(&db, "ses_test", 10).unwrap();
        assert!(msgs[0].contains("teams-cli"));
    }
}
