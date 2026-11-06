//! Session transcript import: structured digests (default) and optional legacy snippet ingest.

mod digest;
mod discover;
mod opencode;
mod parse;
mod types;

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::db::store::{content_hash, looks_like_secret, word_count, BrainStore};
use crate::embed::Embedder;

pub use digest::count_stored_digests_by_source;
pub use discover::{opencode_db_path, session_scan_home};
pub use types::{SessionSource, SessionTranscript};

const META_PREFIX: &str = "session_ingest:";
const MAX_FILES_PER_RUN: usize = 150;
const MAX_USER_MSGS_PER_FILE: usize = 12;
const MAX_WORDS: usize = 50;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct SessionIngestReport {
    pub digests_stored: usize,
    pub legacy_stored: usize,
    pub by_source: HashMap<String, usize>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionDiscoverReport {
    pub total: usize,
    pub by_source: HashMap<String, usize>,
}

/// Default session import: structured digests. Legacy snippet ingest when enabled in config.
pub fn ingest_sessions(store: &BrainStore, embedder: &Embedder, config: &Config) -> Result<usize> {
    let report =
        ingest_sessions_filtered(store, embedder, config, &[], config.session_ingest_legacy)?;
    Ok(report.digests_stored + report.legacy_stored)
}

pub fn ingest_sessions_filtered(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
    sources: &[SessionSource],
    legacy: bool,
) -> Result<SessionIngestReport> {
    if !config.session_ingest_enabled {
        return Ok(SessionIngestReport::default());
    }

    let mut report = SessionIngestReport::default();
    if config.session_digest_enabled {
        report.digests_stored = if sources.is_empty() {
            digest::ingest_session_digests(store, embedder, config)?
        } else {
            digest::ingest_session_digests_filtered(store, embedder, config, sources)?
        };
    }
    if legacy && config.session_ingest_legacy {
        report.legacy_stored = ingest_legacy_sessions_filtered(store, embedder, config, sources)?;
    }
    Ok(report)
}

pub fn discover_report(config: &Config) -> Result<SessionDiscoverReport> {
    let sessions = discover::discover_sessions(config)?;
    let counts = discover::count_by_source(&sessions);
    Ok(SessionDiscoverReport {
        total: sessions.len(),
        by_source: counts
            .into_iter()
            .map(|(k, v)| (k.as_str().to_string(), v))
            .collect(),
    })
}

pub fn ingest_legacy_sessions(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
) -> Result<usize> {
    ingest_legacy_sessions_filtered(
        store,
        embedder,
        config,
        &[],
    )
}

fn ingest_legacy_sessions_filtered(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
    sources: &[SessionSource],
) -> Result<usize> {
    if !config.session_ingest_enabled {
        return Ok(0);
    }

    let mut sessions = discover::discover_sessions_filtered(config, sources)?;
    sessions.retain(|s| s.jsonl_path.is_some());
    sessions.sort_by(|a, b| {
        modified_key(&a.jsonl_path)
            .cmp(&modified_key(&b.jsonl_path))
            .reverse()
    });

    let mut ingested = 0;
    for session in sessions.into_iter().take(MAX_FILES_PER_RUN) {
        if let Some(path) = &session.jsonl_path {
            ingested += ingest_legacy_file_if_changed(store, embedder, &session.source, path)?;
        }
    }
    Ok(ingested)
}

fn modified_key(path: &Option<std::path::PathBuf>) -> SystemTime {
    path.as_ref()
        .and_then(|p| fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn ingest_legacy_file_if_changed(
    store: &BrainStore,
    embedder: &Embedder,
    source: &SessionSource,
    path: &Path,
) -> Result<usize> {
    let raw = fs::read_to_string(path)?;
    let digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let key = format!("{META_PREFIX}{}", path.display());

    if store.get_meta(&key)?.as_deref() == Some(digest.as_str()) {
        return Ok(0);
    }

    let source_label = source.as_str();
    let mut count = 0;
    let reader = BufReader::new(raw.as_bytes());
    for (idx, line) in reader.lines().enumerate() {
        if count >= MAX_USER_MSGS_PER_FILE {
            break;
        }
        let Ok(line) = line else { continue };
        let Some(text) = parse::parse_user_line(*source, &line) else {
            continue;
        };
        if text.len() < 20 || looks_like_secret(&text) {
            continue;
        }
        let fact = truncate_words(&text, MAX_WORDS);
        if word_count(&fact) < 4 {
            continue;
        }

        let topic = format!(
            "legacy-{source_label}-{:x}",
            idx as u64 ^ digest.len() as u64
        );
        let hash = content_hash(&fact);
        let embedding = embedder.embed_one(&format!("{topic} {fact}"))?;
        let res = store.store_fact(
            &topic,
            &fact,
            "global",
            None,
            0.75,
            "session_import",
            &hash,
            &embedding,
            "positive",
        )?;
        if res.stored {
            count += 1;
        }
    }

    store.set_meta(&key, &digest)?;
    Ok(count)
}

pub(crate) fn extract_user_text(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("role").and_then(|r| r.as_str()) != Some("user") {
        return None;
    }

    let mut parts = Vec::new();
    if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
        for item in content {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(text);
                }
            }
        }
    }

    let joined = parts.join("\n");
    let cleaned = strip_user_query_tags(&joined).trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn strip_user_query_tags(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(start) = out.find("<user_query>") {
        if let Some(end) = out.find("</user_query>") {
            let inner = &out[start + 12..end];
            return inner.trim().to_string();
        }
    }
    for tag in [
        "<manually_attached_skills>",
        "<user_info>",
        "<rules>",
        "<agent_skills>",
    ] {
        if let Some(pos) = out.find(tag) {
            out = out[..pos].to_string();
        }
    }
    out
}

fn truncate_words(text: &str, max_words: usize) -> String {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.len() <= max_words {
        text.to_string()
    } else {
        words[..max_words].join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_user_query_text() {
        let line = r#"{"role":"user","message":{"content":[{"type":"text","text":"<user_query>\nuse vitest not jest\n</user_query>"}]}}"#;
        let text = extract_user_text(line).unwrap();
        assert!(text.contains("vitest"));
        assert!(!text.contains("user_query"));
    }

    #[test]
    fn truncates_long_facts() {
        let words: String = (0..60)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(truncate_words(&words, 50).split_whitespace().count(), 50);
    }
}
