//! Structured session digests — one summary fact per conversation session.

use std::collections::HashMap;
use std::fs;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::db::store::{content_hash, looks_like_secret, word_count, BrainStore};
use crate::embed::Embedder;
use crate::sessions::discover;
use crate::sessions::opencode;
use crate::sessions::types::{SessionSource, SessionTranscript};

const MAX_USER_MSGS: usize = 20;
const MAX_DIGEST_WORDS: usize = 80;

pub fn ingest_session_digests(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
) -> Result<usize> {
    ingest_session_digests_filtered(store, embedder, config, &[])
}

pub fn ingest_session_digests_filtered(
    store: &BrainStore,
    embedder: &Embedder,
    config: &Config,
    sources: &[SessionSource],
) -> Result<usize> {
    if !config.session_ingest_enabled || !config.session_digest_enabled {
        return Ok(0);
    }

    let sessions = discover::discover_sessions_filtered(config, sources)?;
    let mut ingested = 0usize;
    for session in sessions {
        if ingest_session(store, embedder, &session)? {
            ingested += 1;
        }
    }
    Ok(ingested)
}

fn ingest_session(
    store: &BrainStore,
    embedder: &Embedder,
    session: &SessionTranscript,
) -> Result<bool> {
    let content_key = session_content_hash(session)?;
    if store.get_meta(&session.meta_key)?.as_deref() == Some(content_key.as_str()) {
        return Ok(false);
    }

    let user_msgs = opencode::load_user_messages(session, MAX_USER_MSGS)?;
    if user_msgs.is_empty() {
        store.set_meta(&session.meta_key, &content_key)?;
        return Ok(false);
    }

    let digest_text = build_digest(session.source.as_str(), &user_msgs);
    if word_count(&digest_text) < 6 || looks_like_secret(&digest_text) {
        return Ok(false);
    }

    let topic = session.digest_topic();
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

    store.set_meta(&session.meta_key, &content_key)?;
    Ok(res.stored)
}

fn session_content_hash(session: &SessionTranscript) -> Result<String> {
    if let Some(path) = &session.jsonl_path {
        let raw = fs::read_to_string(path)?;
        return Ok(format!("{:x}", Sha256::digest(raw.as_bytes())));
    }
    if let Some(db) = &session.opencode_db {
        let msgs = opencode::extract_user_messages(db, &session.session_id, MAX_USER_MSGS)?;
        let joined = msgs.join("\n");
        return Ok(format!("{:x}", Sha256::digest(joined.as_bytes())));
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(session.session_id.as_bytes())
    ))
}

fn build_digest(source: &str, messages: &[String]) -> String {
    let topics: Vec<String> = messages
        .iter()
        .take(5)
        .map(|m| m.split_whitespace().take(8).collect::<Vec<_>>().join(" "))
        .collect();

    let last = messages.last().map(|s| s.as_str()).unwrap_or("");
    let tail = truncate_words(last, 30);

    truncate_words(
        &format!(
            "Retrieved only via route_task. Session digest ({source}, {} turns): {}. Latest focus: {}",
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

pub fn count_stored_digests_by_source(store: &BrainStore) -> Result<HashMap<String, usize>> {
    let facts = store.list_facts(500)?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for fact in facts {
        let topic = fact.get("topic").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(rest) = topic.strip_prefix("session-digest-") {
            let source = rest.split('-').next().unwrap_or("unknown");
            *counts.entry(source.to_string()).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::Embedder;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> Config {
        let home = dir.path().to_path_buf();
        Config {
            home: home.clone(),
            data_dir: home.join("data"),
            logs_dir: home.join("logs"),
            db_path: home.join("data").join("brain.db"),
            vectors_path: home.join("data").join("vectors.bin"),
            turn_ttl_secs: 60,
            auto_capture_enabled: true,
            session_ingest_enabled: true,
            session_digest_enabled: true,
            session_ingest_legacy: false,
            session_max_age_days: 90,
            prewarm_on_bootstrap: false,
            bootstrap_background: false,
            embedding_cache_enabled: true,
            bm25_fast_path_enabled: true,
            session_ingest_background: false,
            turn_cache_ignore_open_files: true,
            embedding_model: "mini".into(),
            bootstrap_startup_delay_secs: 0,
            bootstrap_interval_secs: 0,
            auto_update_startup_delay_secs: 0,
            session_ingest_delay_secs: 0,
            session_ingest_route_interval_secs: 0,
            route_briefing_enabled: false,
            route_briefing_stderr: false,
            mcp_gate_enabled: false,
            mcp_gate_ttl_secs: 600,
            session_stickiness_secs: 0,
            ann_enabled: true,
            ann_min_index: 1_500,
            ann_top_k: 100,
            workflow_dirs: vec![],
        }
    }

    #[test]
    fn per_session_topics_do_not_collide() {
        let a = SessionTranscript::jsonl(
            std::path::PathBuf::from("/tmp/a.jsonl"),
            SessionSource::Gemini,
            "session-a".into(),
        );
        let b = SessionTranscript::jsonl(
            std::path::PathBuf::from("/tmp/b.jsonl"),
            SessionSource::Gemini,
            "session-b".into(),
        );
        assert_ne!(a.digest_topic(), b.digest_topic());
    }

    #[test]
    fn ingests_gemini_jsonl_digest() {
        let dir = TempDir::new().unwrap();
        std::env::set_var("AGENT_BRAIN_SESSION_HOME", dir.path());
        let config = test_config(&dir);
        config.ensure_dirs().unwrap();
        let store = BrainStore::open(&config.db_path).unwrap();
        let embedder = Embedder::deterministic();

        let gemini_root = dir
            .path()
            .join(".gemini/cli/brain/uuid-1/.system_generated/logs");
        std::fs::create_dir_all(&gemini_root).unwrap();
        let transcript = gemini_root.join("transcript.jsonl");
        std::fs::write(
            &transcript,
            r#"{"type":"USER_INPUT","content":"<USER_REQUEST>\nimplement agent-brain session ingest for gemini transcripts\n</USER_REQUEST>"}"#,
        )
        .unwrap();

        let n = ingest_session_digests(&store, &embedder, &config).unwrap();
        assert_eq!(n, 1);
        let facts = store.list_facts(10).unwrap();
        assert!(facts.iter().any(|f| f["topic"]
            .as_str()
            .unwrap_or("")
            .starts_with("session-digest-gemini-")));
        std::env::remove_var("AGENT_BRAIN_SESSION_HOME");
    }
}
