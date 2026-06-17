//! Ingest allowlisted documentation URLs into the skill index + memory.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::db::store::{content_hash, word_count, BrainStore};
use crate::embed::Embedder;
use crate::engine::Engine;
use crate::index::skill_index_text;
use crate::settings::DocsSettings;
use crate::types::ItemType;

use super::allowlist::assert_url_allowed;
use super::fetch::{extract_title, fetch_url, html_to_text};

#[derive(Debug, Clone, serde::Serialize)]
pub struct LearnReport {
    pub url: String,
    pub domain: String,
    pub topic: String,
    pub bytes_fetched: usize,
    pub words: usize,
    pub chunks_indexed: usize,
    pub memory_stored: bool,
    pub cached_path: String,
    pub dry_run: bool,
}

pub fn learn_from_url(
    engine: &Engine,
    url: &str,
    topic: Option<&str>,
    dry_run: bool,
) -> Result<LearnReport> {
    let settings = crate::settings::AgentBrainSettings::load(&engine.config.home);
    if !settings.docs.enabled {
        anyhow::bail!("docs learning is disabled — set docs.enabled: true in ~/.agent_brain/config.yaml");
    }
    learn_with_settings(
        &engine.store,
        &engine.embedder,
        &engine.config.home,
        &settings.docs,
        url,
        topic,
        dry_run,
    )
}

pub fn learn_with_settings(
    store: &BrainStore,
    embedder: &Embedder,
    home: &Path,
    settings: &DocsSettings,
    url: &str,
    topic: Option<&str>,
    dry_run: bool,
) -> Result<LearnReport> {
    let url = url.trim();
    let domain = assert_url_allowed(url, &settings.allowed_domains)?;
    let bytes = fetch_url(url, settings.max_bytes)?;
    let bytes_fetched = bytes.len();
    let body = String::from_utf8_lossy(&bytes);
    let plain = html_to_text(&body);
    if plain.split_whitespace().count() < 20 {
        anyhow::bail!("page too short after extraction — not enough documentation text");
    }

    let title = extract_title(&body);
    let topic = topic
        .map(slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| infer_topic(url, title.as_deref()));
    let cached_path = cache_path(home, url, &topic)?;
    if !dry_run {
        fs::write(&cached_path, format!("# {topic}\n\nSource: {url}\n\n{plain}\n"))
            .with_context(|| format!("write {}", cached_path.display()))?;
    }

    let chunks = chunk_words(&plain, settings.chunk_words, settings.max_chunks);
    let mut chunks_indexed = 0usize;
    let mut memory_stored = false;
    if !dry_run {
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_topic = if chunks.len() == 1 {
                topic.clone()
            } else {
                format!("{topic}-part-{}", idx + 1)
            };
            let skill_body = format!(
                "# {chunk_topic}\n\nLearned from {url}\n\nUse when routing questions about {topic}.\n\n{chunk}"
            );
            let text = skill_index_text(&skill_body, &chunk_topic);
            let hash = content_hash(&text);
            let embedding = embedder.embed_one(&format!("{chunk_topic} {text}"))?;
            store.upsert_indexed_item(
                ItemType::Skill,
                &chunk_topic,
                &text,
                url,
                "global",
                None,
                &hash,
                Some(&embedding),
            )?;
            chunks_indexed += 1;
        }
        store.bump_index_version()?;

        let summary = build_summary(&topic, url, &plain, settings.summary_words);
        if word_count(&summary) >= 6 {
            let hash = content_hash(&summary);
            let embedding = embedder.embed_one(&format!("learned-url-{topic} {summary}"))?;
            let res = store.store_fact(
                &format!("learned-url-{topic}"),
                &summary,
                "global",
                None,
                0.88,
                "learned_url",
                &hash,
                &embedding,
                "positive",
            )?;
            memory_stored = res.stored || res.deduplicated;
        }
    }

    Ok(LearnReport {
        url: url.to_string(),
        domain,
        topic,
        bytes_fetched,
        words: plain.split_whitespace().count(),
        chunks_indexed,
        memory_stored,
        cached_path: cached_path.display().to_string(),
        dry_run,
    })
}

fn build_summary(topic: &str, url: &str, plain: &str, max_words: usize) -> String {
    let snippet: String = plain
        .split_whitespace()
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ");
    format!("Learned docs `{topic}` from {url}: {snippet}")
}

fn chunk_words(text: &str, chunk_words: usize, max_chunks: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let chunk_words = chunk_words.max(80).min(600);
    let max_chunks = max_chunks.max(1).min(12);
    let mut chunks = Vec::new();
    for chunk in words.chunks(chunk_words).take(max_chunks) {
        chunks.push(chunk.join(" "));
    }
    chunks
}

fn cache_path(home: &Path, url: &str, topic: &str) -> Result<PathBuf> {
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    let dir = home.join("learned").join(topic);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir.join(format!("{digest}.md")))
}

fn infer_topic(url: &str, title: Option<&str>) -> String {
    if let Some(title) = title {
        let slug = slugify(title);
        if !slug.is_empty() {
            return slug;
        }
    }
    let path = url
        .strip_prefix("https://")
        .unwrap_or(url)
        .split('/')
        .skip(1)
        .filter(|s| !s.is_empty())
        .take(3)
        .map(slugify)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if path.is_empty() {
        "learned-doc".into()
    } else {
        path
    }
}

fn slugify(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_long_text() {
        let words: Vec<String> = (0..500).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_words(&text, 200, 5);
        assert!(chunks.len() <= 5);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn slugify_topic() {
        assert_eq!(slugify("Next.js App Router"), "next-js-app-router");
    }
}
