//! HACK: one-off ingest of legacy Cursor/Codex chat transcripts into memory.
//! Remove this module when proper session digests land (planned 0.4.x).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::config::Config;
use crate::db::store::{content_hash, looks_like_secret, word_count, BrainStore};
use crate::embed::Embedder;

const META_PREFIX: &str = "session_ingest:";
const MAX_FILES_PER_RUN: usize = 150;
const MAX_USER_MSGS_PER_FILE: usize = 12;
const MAX_WORDS: usize = 50;

pub fn ingest_legacy_sessions(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
) -> Result<usize> {
    if !config.session_ingest_enabled {
        return Ok(0);
    }

    let mut paths = discover_session_files(config)?;
    paths.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    paths.reverse();

    let mut ingested = 0;
    for path in paths.into_iter().take(MAX_FILES_PER_RUN) {
        ingested += ingest_file_if_changed(store, embedder, &path)?;
    }
    Ok(ingested)
}

fn discover_session_files(config: &Config) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return Ok(paths);
    };

    let cursor_glob = format!(
        "{}/.cursor/projects/**/agent-transcripts/**/*.jsonl",
        home.display()
    );
    if let Ok(entries) = glob::glob(&cursor_glob) {
        for entry in entries.flatten() {
            if is_recent_enough(&entry, config.session_max_age_days) {
                paths.push(entry);
            }
        }
    }

    let codex_root = home.join(".codex/sessions");
    if codex_root.is_dir() {
        for entry in WalkDir::new(&codex_root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path().to_path_buf();
            if path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                && is_recent_enough(&path, config.session_max_age_days)
            {
                paths.push(path);
            }
        }
    }

    Ok(paths)
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

fn ingest_file_if_changed(
    store: &BrainStore,
    embedder: &Embedder,
    path: &Path,
) -> Result<usize> {
    let raw = fs::read_to_string(path)?;
    let digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let key = format!("{META_PREFIX}{}", path.display());

    if store.get_meta(&key)?.as_deref() == Some(digest.as_str()) {
        return Ok(0);
    }

    let source_label = if path.to_string_lossy().contains(".cursor/") {
        "cursor"
    } else {
        "codex"
    };

    let mut count = 0;
    let reader = BufReader::new(raw.as_bytes());
    for (idx, line) in reader.lines().enumerate() {
        if count >= MAX_USER_MSGS_PER_FILE {
            break;
        }
        let Ok(line) = line else { continue };
        let Some(text) = extract_user_text(&line) else {
            continue;
        };
        if text.len() < 20 || looks_like_secret(&text) {
            continue;
        }
        let fact = truncate_words(&text, MAX_WORDS);
        if word_count(&fact) < 4 {
            continue;
        }

        let topic = format!("legacy-{source_label}-{:x}", idx as u64 ^ digest.len() as u64);
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
        )?;
        if res.stored {
            count += 1;
        }
    }

    store.set_meta(&key, &digest)?;
    Ok(count)
}

fn extract_user_text(line: &str) -> Option<String> {
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
        let words: String = (0..60).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ");
        assert_eq!(truncate_words(&words, 50).split_whitespace().count(), 50);
    }
}
