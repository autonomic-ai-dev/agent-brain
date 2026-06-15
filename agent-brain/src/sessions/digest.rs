//! Structured session digests — one summary fact per transcript file.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::db::store::{content_hash, looks_like_secret, word_count, BrainStore};
use crate::embed::Embedder;
use crate::sessions::discover_session_files;

const META_PREFIX: &str = "session_digest:";
const MAX_USER_MSGS: usize = 20;
const MAX_DIGEST_WORDS: usize = 80;

pub fn ingest_session_digests(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
) -> Result<usize> {
    if !config.session_ingest_enabled || !config.session_digest_enabled {
        return Ok(0);
    }

    let paths = discover_session_files(config)?;
    let mut ingested = 0usize;
    for path in paths {
        if ingest_file(store, embedder, &path)? {
            ingested += 1;
        }
    }
    Ok(ingested)
}

fn ingest_file(store: &BrainStore, embedder: &Embedder, path: &Path) -> Result<bool> {
    let raw = fs::read_to_string(path)?;
    let digest_key = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let meta_key = format!("{META_PREFIX}{}", path.display());

    if store.get_meta(&meta_key)?.as_deref() == Some(digest_key.as_str()) {
        return Ok(false);
    }

    let source = if path.to_string_lossy().contains(".cursor/") {
        "cursor"
    } else {
        "codex"
    };

    let user_msgs = extract_user_messages(&raw);
    if user_msgs.is_empty() {
        store.set_meta(&meta_key, &digest_key)?;
        return Ok(false);
    }

    let digest_text = build_digest(source, &user_msgs);
    if word_count(&digest_text) < 6 || looks_like_secret(&digest_text) {
        return Ok(false);
    }

    let topic = format!("session-digest-{source}");
    let hash = content_hash(&digest_text);
    let embedding = embedder.embed_one(&format!("{topic} {digest_text}"))?;
    let res = store.store_fact(
        &topic,
        &digest_text,
        "global",
        None,
        0.85,
        "session_digest",
        &hash,
        &embedding,
        "positive",
    )?;

    store.set_meta(&meta_key, &digest_key)?;
    Ok(res.stored)
}

fn extract_user_messages(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in BufReader::new(raw.as_bytes()).lines().map_while(Result::ok) {
        if out.len() >= MAX_USER_MSGS {
            break;
        }
        if let Some(text) = super::extract_user_text(&line) {
            if text.len() >= 20 && !looks_like_secret(&text) {
                out.push(text);
            }
        }
    }
    out
}

fn build_digest(source: &str, messages: &[String]) -> String {
    let topics: Vec<String> = messages
        .iter()
        .take(5)
        .map(|m| {
            m.split_whitespace()
                .take(8)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();

    let last = messages.last().map(|s| s.as_str()).unwrap_or("");
    let tail = truncate_words(last, 30);

    truncate_words(
        &format!(
            "Session digest ({source}, {} turns): {}. Latest focus: {}",
            messages.len(),
            topics.join(" | "),
            tail
        ),
        MAX_DIGEST_WORDS,
    )
}

fn truncate_words(text: &str, max_words: usize) -> String {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.len() <= max_words {
        text.to_string()
    } else {
        words[..max_words].join(" ")
    }
}
