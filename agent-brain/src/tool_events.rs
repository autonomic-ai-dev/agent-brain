//! Hook-emitted tool events (JSONL) merged into operator stats.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::db::store::{BrainStore, ToolEventRecord};

pub fn hook_events_path(home: &Path) -> std::path::PathBuf {
    home.join("hooks/tool_events.jsonl")
}

pub fn append_hook_event(home: &Path, record: &ToolEventRecord) -> Result<()> {
    let path = hook_events_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(record)?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(file, "{line}")?;
    Ok(())
}

pub fn ingest_hook_events_since(store: &BrainStore, home: &Path, since_ms: i64) -> Result<usize> {
    let path = hook_events_path(home);
    if !path.exists() {
        return Ok(0);
    }
    let file = fs::File::open(&path)?;
    let reader = BufReader::new(file);
    let mut ingested = 0usize;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: ToolEventRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if record.timestamp < since_ms {
            continue;
        }
        let id = format!("{:x}", Sha256::digest(line.as_bytes()));
        store.insert_tool_log(
            &id,
            &record.tool_name,
            record.path.as_deref(),
            record.detail.as_deref(),
            record.tokens_used,
            record.tokens_saved,
            record.savings_pct,
            record.must_apply_active,
            record.phase.as_deref(),
            None,
        )?;
        ingested += 1;
    }
    Ok(ingested)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 24 * 3600 * 1000;

    fn record(timestamp: i64) -> ToolEventRecord {
        ToolEventRecord {
            timestamp,
            tool_name: "read_file_head".into(),
            path: Some("src/lib.rs".into()),
            detail: None,
            tokens_used: 120,
            tokens_saved: Some(480),
            savings_pct: Some(80.0),
            must_apply_active: false,
            phase: None,
        }
    }

    fn open_store(home: &Path) -> BrainStore {
        BrainStore::open(&home.join("brain.db")).unwrap()
    }

    fn tool_calls_since(store: &BrainStore, since_ms: i64) -> usize {
        store.tool_log_stats_since(since_ms).unwrap().0
    }

    #[test]
    fn ingest_leaves_hook_log_for_other_readers() {
        let home = tempfile::tempdir().unwrap();
        let store = open_store(home.path());
        let now = chrono::Utc::now().timestamp_millis();
        append_hook_event(home.path(), &record(now)).unwrap();

        assert_eq!(
            ingest_hook_events_since(&store, home.path(), now - DAY_MS).unwrap(),
            1
        );

        let remaining = fs::read_to_string(hook_events_path(home.path())).unwrap();
        assert_eq!(remaining.lines().count(), 1);
    }

    #[test]
    fn repeated_ingest_does_not_double_count_tool_calls() {
        let home = tempfile::tempdir().unwrap();
        let store = open_store(home.path());
        let now = chrono::Utc::now().timestamp_millis();
        append_hook_event(home.path(), &record(now)).unwrap();

        ingest_hook_events_since(&store, home.path(), now - DAY_MS).unwrap();
        ingest_hook_events_since(&store, home.path(), now - DAY_MS).unwrap();

        assert_eq!(tool_calls_since(&store, now - DAY_MS), 1);
    }

    #[test]
    fn narrow_window_ingest_keeps_events_for_wider_window() {
        let home = tempfile::tempdir().unwrap();
        let store = open_store(home.path());
        let now = chrono::Utc::now().timestamp_millis();
        append_hook_event(home.path(), &record(now - 3 * DAY_MS)).unwrap();
        append_hook_event(home.path(), &record(now)).unwrap();

        ingest_hook_events_since(&store, home.path(), now - DAY_MS).unwrap();
        ingest_hook_events_since(&store, home.path(), now - 7 * DAY_MS).unwrap();

        assert_eq!(tool_calls_since(&store, now - 7 * DAY_MS), 2);
    }

    #[test]
    fn only_bounded_read_tools_count_as_tool_calls() {
        let home = tempfile::tempdir().unwrap();
        let store = open_store(home.path());
        for (id, tool) in [
            ("a", "read_file_head"),
            ("b", "grep_search"),
            ("c", "bash"),
            ("d", "edit"),
            ("e", "cursor_read"),
        ] {
            store
                .insert_tool_log(id, tool, None, None, 10, None, None, false, None, None)
                .unwrap();
        }

        assert_eq!(tool_calls_since(&store, 0), 2);
    }
}
