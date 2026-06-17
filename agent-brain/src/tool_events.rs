//! Hook-emitted tool events (JSONL) merged into operator stats.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};

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
        let id = uuid::Uuid::new_v4().to_string();
        store.insert_tool_log(
            &id,
            &record.tool_name,
            record.path.as_deref(),
            record.tokens_used,
            record.tokens_saved,
            record.savings_pct,
            record.must_apply_active,
            record.phase.as_deref(),
            None,
        )?;
        ingested += 1;
    }
    if ingested > 0 {
        let _ = fs::rename(&path, path.with_extension("jsonl.ingested"));
    }
    Ok(ingested)
}
