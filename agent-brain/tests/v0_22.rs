//! v0.22 — temporal store_memory params and observation engine.

use std::sync::Arc;

use agent_brain::db::store::{content_hash, BrainStore, FactTemporal};
use agent_brain::embed::{deterministic_embedding, Embedder};
use agent_brain::engine::Engine;
use agent_brain::observation::{run_observations, ObservationConfig};
use agent_brain::types::RouteLimits;
use agent_brain::{config::Config, temporal};
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
        session_stickiness_secs: 0,
        ann_enabled: true,
        ann_min_index: 1_500,
        ann_top_k: 100,
        workflow_dirs: vec![],
    }
}

#[test]
fn temporal_invalid_at_excludes_fact_from_active_queries() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Deprecated Jest preference";
    let emb = deterministic_embedding(text);
    let now = chrono::Utc::now().timestamp_millis();

    store
        .store_fact_full(
            "testing",
            text,
            "project",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "negative",
            None,
            Some(&FactTemporal {
                valid_from: Some(now - 10_000),
                invalid_at: Some(now - 1),
            }),
        )
        .unwrap();

    assert!(!temporal::is_fact_active(now, now - 10_000, Some(now - 1)));

    let active = store
        .get_active_fact_by_topic("testing", "project", None)
        .unwrap();
    assert!(active.is_none());
}

#[test]
fn temporal_valid_from_defers_fact_activation() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Future Vitest mandate";
    let emb = deterministic_embedding(text);
    let now = chrono::Utc::now().timestamp_millis();

    store
        .store_fact_full(
            "testing",
            text,
            "project",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "positive",
            None,
            Some(&FactTemporal {
                valid_from: Some(now + 60_000),
                invalid_at: None,
            }),
        )
        .unwrap();

    let active = store
        .get_active_fact_by_topic("testing", "project", None)
        .unwrap();
    assert!(active.is_none());
}

#[test]
fn observation_surfaces_must_apply_when_tags_match() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let embedder = Embedder::deterministic();
    let emb = vec![0.03; 384];

    for i in 0..3 {
        let fact = format!("Prefer Vitest over Jest for component tests {i}");
        store
            .store_fact_full(
                "testing",
                &fact,
                "project",
                None,
                0.9,
                "agent",
                &content_hash(&fact),
                &emb,
                "positive",
                Some(r#"["phase:implementing"]"#),
                None,
            )
            .unwrap();
    }

    let report = run_observations(&store, &embedder, &ObservationConfig::default(), false).unwrap();
    assert_eq!(report.synthesized, 1);

    let engine = Engine::new_with_store(config.clone(), Arc::clone(&store)).unwrap();
    let resp = engine
        .route_task(
            "add vitest tests for react components",
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
        resp.must_apply.iter().any(|m| m.topic.starts_with("obs/"))
            || resp
                .relevant_memory
                .iter()
                .any(|m| m.topic.starts_with("obs/")),
        "observation memory should surface in route output"
    );
}
