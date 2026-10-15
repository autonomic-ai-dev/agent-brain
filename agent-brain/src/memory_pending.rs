//! Durable fallback when agent-brain MCP is flaky: enqueue memory locally, then flush with retries.
//!
//! Use when Cursor reports `MCP error -32000: Connection closed` on `store_memory` / reads:
//! 1. Retry the MCP tool a few times (agent rule).
//! 2. `agent-brain memory enqueue ...` so the fact is not lost.
//! 3. `agent-brain memory flush-pending` (or `--watch`) to apply when the brain is reachable via CLI.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::db::store::{content_hash, looks_like_secret, word_count, BrainStore};
use crate::embed::Embedder;

const PENDING_FILE: &str = "pending_memory.jsonl";
pub const DEFAULT_RETRIES: u32 = 3;
pub const DEFAULT_BACKOFF_MS: u64 = 400;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMemory {
    pub topic: String,
    pub fact: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub enqueued_at: i64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

fn default_scope() -> String {
    "project".to_string()
}

fn default_confidence() -> f64 {
    0.9
}

fn pending_path(home: &Path) -> PathBuf {
    home.join(PENDING_FILE)
}

pub fn enqueue(
    home: &Path,
    topic: &str,
    fact: &str,
    scope: &str,
    scope_key: Option<&str>,
    confidence: f64,
) -> Result<PathBuf> {
    let topic = topic.trim();
    let fact = fact.trim();
    if topic.is_empty() || fact.is_empty() {
        bail!("topic and fact are required");
    }
    if looks_like_secret(fact) {
        bail!("prohibited content");
    }
    if word_count(fact) > 50 {
        bail!("fact exceeds 50 words");
    }
    fs::create_dir_all(home).context("create brain home")?;
    let path = pending_path(home);
    let entry = PendingMemory {
        topic: topic.to_string(),
        fact: fact.to_string(),
        scope: if scope.is_empty() {
            "project".into()
        } else {
            scope.to_string()
        },
        scope_key: scope_key.map(|s| s.to_string()),
        confidence: if confidence <= 0.0 { 0.9 } else { confidence },
        enqueued_at: chrono::Utc::now().timestamp_millis(),
        attempts: 0,
        last_error: None,
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    Ok(path)
}

/// Write a fact immediately through the local DB (no MCP).
pub fn put_now(
    store: &BrainStore,
    embedder: &Embedder,
    topic: &str,
    fact: &str,
    scope: &str,
    scope_key: Option<&str>,
    confidence: f64,
) -> Result<serde_json::Value> {
    if looks_like_secret(fact) {
        bail!("prohibited content");
    }
    if word_count(fact) > 50 {
        bail!("fact exceeds 50 words");
    }
    let hash = content_hash(fact);
    let embedding = embedder.embed_one(&format!("{topic} {fact}"))?;
    let res = store.store_fact_full(
        topic,
        fact,
        scope,
        scope_key,
        if confidence <= 0.0 { 0.9 } else { confidence },
        "agent",
        &hash,
        &embedding,
        "positive",
        None,
        None,
    )?;
    Ok(serde_json::json!({
        "stored": res.stored,
        "deduplicated": res.deduplicated,
        "id": res.id,
        "via": "cli",
    }))
}

#[derive(Debug, Default, Serialize)]
pub struct FlushReport {
    pub applied: usize,
    pub failed: usize,
    pub remaining: usize,
    pub errors: Vec<String>,
}

/// Apply pending queue with per-item retries. Survives MCP outages by using the local binary/DB.
pub fn flush_pending(
    home: &Path,
    store: &BrainStore,
    embedder: &Embedder,
    retries: u32,
    backoff_ms: u64,
) -> Result<FlushReport> {
    let path = pending_path(home);
    if !path.is_file() {
        return Ok(FlushReport::default());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut remaining: Vec<PendingMemory> = Vec::new();
    let mut report = FlushReport::default();

    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut entry: PendingMemory = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(err) => {
                report.failed += 1;
                report.errors.push(format!("parse: {err}"));
                continue;
            }
        };

        let mut last_err = None;
        let attempts = retries.max(1);
        let mut ok = false;
        for i in 0..attempts {
            entry.attempts += 1;
            match put_now(
                store,
                embedder,
                &entry.topic,
                &entry.fact,
                &entry.scope,
                entry.scope_key.as_deref(),
                entry.confidence,
            ) {
                Ok(_) => {
                    ok = true;
                    report.applied += 1;
                    break;
                }
                Err(err) => {
                    last_err = Some(err.to_string());
                    if i + 1 < attempts {
                        thread::sleep(Duration::from_millis(backoff_ms.saturating_mul(1 + i as u64)));
                    }
                }
            }
        }
        if !ok {
            entry.last_error = last_err.clone();
            report.failed += 1;
            if let Some(e) = last_err {
                report.errors.push(format!("{}: {e}", entry.topic));
            }
            remaining.push(entry);
        }
    }

    if remaining.is_empty() {
        let _ = fs::remove_file(&path);
    } else {
        let mut out = String::new();
        for e in &remaining {
            out.push_str(&serde_json::to_string(e)?);
            out.push('\n');
        }
        fs::write(&path, out)?;
    }
    report.remaining = remaining.len();
    Ok(report)
}

/// Watch loop: flush, sleep, repeat (for a shell/task runner while MCP is flaky).
pub fn flush_watch(
    home: &Path,
    store: &BrainStore,
    embedder: &Embedder,
    retries: u32,
    backoff_ms: u64,
    interval_secs: u64,
    max_rounds: u32,
) -> Result<FlushReport> {
    let mut total = FlushReport::default();
    for round in 0..max_rounds.max(1) {
        let report = flush_pending(home, store, embedder, retries, backoff_ms)?;
        total.applied += report.applied;
        total.failed += report.failed;
        total.remaining = report.remaining;
        total.errors.extend(report.errors);
        if total.remaining == 0 {
            break;
        }
        if round + 1 < max_rounds {
            thread::sleep(Duration::from_secs(interval_secs.max(1)));
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn enqueue_and_flush_applies_fact() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        enqueue(home, "topic", "a durable convention fact", "global", None, 0.9).unwrap();
        let store = BrainStore::open(&home.join("brain.db")).unwrap();
        let emb = Embedder::deterministic();
        let report = flush_pending(home, &store, &emb, 2, 1).unwrap();
        assert_eq!(report.applied, 1);
        assert_eq!(report.remaining, 0);
        assert!(!pending_path(home).exists());
    }
}
