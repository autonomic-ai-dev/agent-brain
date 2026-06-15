use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use agent_brain::cache::{fingerprint_open_files, fingerprint_query, CacheKey, TurnCache};
use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, looks_like_secret, BrainStore};
use agent_brain::embed::Embedder;
use agent_brain::engine::Engine;
use agent_brain::tokens::estimate_tokens;
use agent_brain::types::{ItemType, RouteLimits, RouteTaskResponse};
use tempfile::TempDir;

fn test_config(dir: &TempDir) -> Config {
    let home = dir.path().to_path_buf();
    Config {
        home: home.clone(),
        data_dir: home.join("data"),
        db_path: home.join("data").join("brain.db"),
        vectors_path: home.join("data").join("vectors.bin"),
        turn_ttl_secs: 60,
        auto_capture_enabled: true,
    }
}

fn dummy_embedding() -> Vec<f32> {
    vec![0.01; 384]
}

fn shared_embedder() -> Arc<Embedder> {
    static EMBEDDER: OnceLock<Arc<Embedder>> = OnceLock::new();
    Arc::clone(EMBEDDER.get_or_init(|| Arc::new(Embedder::new().expect("embedder init"))))
}

#[test]
fn rejects_secret_patterns() {
    assert!(looks_like_secret("api_key=sk-abcdefghijklmnopqrstuvwxyz"));
    assert!(looks_like_secret("password: hunter2"));
    assert!(looks_like_secret("load from .env file"));
    assert!(!looks_like_secret("prefer Result types over unwrap"));
}

#[test]
fn deduplicates_identical_facts() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = dummy_embedding();
    let hash = content_hash("Use anyhow for errors");

    let first = store
        .store_fact(
            "errors",
            "Use anyhow for errors",
            "project",
            Some("/repo"),
            0.9,
            &hash,
            &emb,
        )
        .unwrap();
    assert!(first.stored);
    assert!(!first.deduplicated);

    let second = store
        .store_fact(
            "errors",
            "Use anyhow for errors",
            "project",
            Some("/repo"),
            0.9,
            &hash,
            &emb,
        )
        .unwrap();
    assert!(!second.stored);
    assert!(second.deduplicated);
    assert_eq!(first.id, second.id);
}

#[test]
fn supersedes_same_topic_facts() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = dummy_embedding();

    let v1 = store
        .store_fact(
            "lint",
            "Run clippy on every PR",
            "project",
            Some("/repo"),
            0.9,
            &content_hash("Run clippy on every PR"),
            &emb,
        )
        .unwrap();
    let v2 = store
        .store_fact(
            "lint",
            "Run clippy and fmt on every PR",
            "project",
            Some("/repo"),
            0.9,
            &content_hash("Run clippy and fmt on every PR"),
            &emb,
        )
        .unwrap();

    assert_ne!(v1.id, v2.id);
    let active: Vec<_> = store.list_facts(10).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["fact"], "Run clippy and fmt on every PR");
}

#[test]
fn turn_cache_returns_hit_on_repeat_query() {
    let cache = TurnCache::new(8, 60);
    let key = CacheKey {
        scope_key: "repo".into(),
        phase: "implement".into(),
        open_files_fp: fingerprint_open_files(&["src/main.rs".into()]),
        query_fp: fingerprint_query("fix the routing bug"),
        index_version: 1,
    };
    let resp = RouteTaskResponse {
        recommended_phase: "implement".into(),
        tokens_budget: 500,
        ..Default::default()
    };
    cache.put(key.clone(), resp);
    let hit = cache.get(&key).expect("cache hit");
    assert!(hit.cache_hit);
}

#[test]
fn truncates_context_to_token_budget() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = shared_embedder();

    for i in 0..20 {
        let text = format!("Long rule number {i} with enough text to consume token budget quickly.");
        let emb = embedder.embed_one(&text).unwrap();
        store
            .upsert_indexed_item(
                ItemType::Rule,
                &format!("rule-{i}"),
                &text,
                &format!("/rules/rule-{i}.md"),
                "global",
                None,
                &content_hash(&text),
                Some(&emb),
            )
            .unwrap();
    }

    let engine = Engine {
        config: config.clone(),
        store: Arc::new(store),
        embedder: embedder.clone(),
        cache: Arc::new(TurnCache::new(8, 60)),
        auto_capture_enabled: true,
    };

    let resp = engine
        .get_context(
            "routing rules",
            None,
            30,
            &[ItemType::Rule],
        )
        .unwrap();

    assert!(resp.truncated || resp.items.len() < 20);
    assert!(resp.tokens_used <= resp.tokens_budget);
}

#[test]
fn route_task_respects_max_tokens() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = shared_embedder();

    for i in 0..10 {
        let text = format!("Agent capability {i} for rust backend work");
        let emb = embedder.embed_one(&text).unwrap();
        store
            .upsert_indexed_item(
                ItemType::Agent,
                &format!("agent-{i}"),
                &text,
                &format!("/agents/agent-{i}.md"),
                "global",
                None,
                &content_hash(&text),
                Some(&emb),
            )
            .unwrap();
    }

    let engine = Engine {
        config,
        store: Arc::new(store),
        embedder: embedder.clone(),
        cache: Arc::new(TurnCache::new(8, 60)),
        auto_capture_enabled: true,
    };

    let resp = engine
        .route_task(
            "implement rust backend",
            Some(PathBuf::from(".").as_path()),
            &[],
            20,
            RouteLimits {
                agents: 5,
                skills: 5,
                rules: 5,
                memory: 5,
            },
        )
        .unwrap();

    assert!(resp.tokens_used <= resp.tokens_budget);
}

#[test]
fn token_estimate_is_reasonable() {
    let text = "hello world";
    assert!(estimate_tokens(text) >= 2);
    assert!(estimate_tokens(text) < 20);
}
