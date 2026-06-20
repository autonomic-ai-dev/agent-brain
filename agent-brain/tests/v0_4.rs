use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_brain::cache::{QueryEmbeddingCache, TurnCache};
use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::db::RouteLatencyStats;
use agent_brain::embed::{deterministic_embedding, Embedder};
use agent_brain::engine::Engine;
use agent_brain::mcp_activity::McpActivity;
use agent_brain::types::{ItemType, RouteLimits};
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
        session_ingest_enabled: false,
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
        ann_enabled: true,
        ann_min_index: 1_500,
        ann_top_k: 100,
            workflow_dirs: vec![],
    }
}

#[test]
fn negative_memory_surfaces_in_must_apply() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let text = "Do not use Jest for this project; prefer Vitest";
    let emb = deterministic_embedding(text);
    store
        .store_fact(
            "testing",
            text,
            "project",
            None,
            0.95,
            "agent",
            &content_hash(text),
            &emb,
            "negative",
        )
        .unwrap();

    let engine = Engine::new_with_store(config.clone(), Arc::clone(&store)).unwrap();

    let resp = engine
        .route_task(
            "configure vitest for react testing",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 0,
                rules: 0,
                memory: 5,
            },
            Some("implementing"),
            None,
        )
        .unwrap();

    assert!(
        !resp.must_apply.is_empty() || !resp.relevant_memory.is_empty(),
        "negative memory should appear in route output"
    );
}

#[test]
fn add_only_preserves_facts_without_supersede_conflict() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = vec![0.01; 384];

    store
        .store_fact(
            "style",
            "Use tabs",
            "global",
            None,
            0.9,
            "agent",
            &content_hash("Use tabs"),
            &emb,
            "positive",
        )
        .unwrap();
    store
        .store_fact(
            "style",
            "Use spaces",
            "global",
            None,
            0.9,
            "agent",
            &content_hash("Use spaces"),
            &emb,
            "positive",
        )
        .unwrap();

    let facts = store.list_facts(10).unwrap();
    assert_eq!(facts.len(), 2);
    let conflicts = store.list_conflicts(10).unwrap();
    assert!(conflicts.is_empty());
}

#[test]
fn retrieval_log_persisted_on_route() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let text = "rust backend patterns for actix web";
    let emb = deterministic_embedding(text);
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "rust-patterns",
            text,
            "/skills/rust/SKILL.md",
            "global",
            None,
            &content_hash(text),
            Some(&emb),
        )
        .unwrap();

    let engine = Engine::new_with_store(config.clone(), Arc::clone(&store)).unwrap();

    let resp = engine
        .route_task(
            "implement rust backend with actix",
            None,
            &[],
            500,
            RouteLimits::default(),
            None,
            None,
        )
        .unwrap();

    let row = store.get_retrieval_log(&resp.log_id).unwrap();
    assert!(row.is_some());
    assert_eq!(row.unwrap().phase, resp.recommended_phase);
}

#[test]
fn context_feedback_records_updates() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();

    let updated = store
        .record_context_feedback(&["item-1".into(), "item-2".into()], true)
        .unwrap();
    assert_eq!(updated, 2);
}
