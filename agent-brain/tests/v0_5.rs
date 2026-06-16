use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_brain::cache::{QueryEmbeddingCache, TurnCache};
use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::db::RouteLatencyStats;
use agent_brain::embed::{deterministic_embedding, Embedder};
use agent_brain::engine::Engine;
use agent_brain::mcp_activity::McpActivity;
use agent_brain::sync::{export_bundle, import_bundle, MergePolicy};
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
        route_briefing_enabled: false,
        route_briefing_stderr: false,
    }
}

fn test_engine(config: &Config, store: Arc<BrainStore>) -> Engine {
    Engine::new_with_store(config.clone(), store).unwrap()
}

#[test]
fn apply_when_surfaces_in_must_apply_when_phase_matches() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let text = "Check race conditions in async handlers before shipping";
    let emb = deterministic_embedding(text);
    store
        .store_fact_full(
            "debugging",
            text,
            "project",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "positive",
            Some(r#"["phase:debugging"]"#),
        )
        .unwrap();

    let engine = test_engine(&config, Arc::clone(&store));
    let resp = engine
        .route_task(
            "debug async race condition in handler",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 0,
                rules: 0,
                memory: 5,
            },
            Some("debugging"),
        )
        .unwrap();

    assert!(
        resp.must_apply.iter().any(|m| m.topic == "debugging"),
        "apply_when match should promote fact to must_apply"
    );
}

#[test]
fn global_vs_project_conflict_emits_warning() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let emb = vec![0.01; 384];

    store
        .store_fact(
            "formatting",
            "Use tabs for indentation",
            "global",
            None,
            0.9,
            "agent",
            &content_hash("Use tabs for indentation"),
            &emb,
            "positive",
        )
        .unwrap();
    store
        .store_fact(
            "formatting",
            "Use two spaces for indentation",
            "project",
            Some("/repo"),
            0.9,
            "agent",
            &content_hash("Use two spaces for indentation"),
            &emb,
            "positive",
        )
        .unwrap();

    let engine = test_engine(&config, Arc::clone(&store));
    let resp = engine
        .route_task(
            "format code indentation style",
            Some(std::path::Path::new("/repo")),
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 0,
                rules: 0,
                memory: 5,
            },
            None,
        )
        .unwrap();

    assert!(
        resp.warnings
            .iter()
            .any(|w| w.topic == "formatting" && w.message.contains("Global vs project")),
        "expected global vs project warning, got {:?}",
        resp.warnings
    );
}

#[test]
fn sync_bundle_round_trip_imports_facts() {
    let src_dir = TempDir::new().unwrap();
    let src_config = test_config(&src_dir);
    src_config.ensure_dirs().unwrap();
    let src_store = BrainStore::open(&src_config.db_path).unwrap();
    let text = "Prefer anyhow for error handling in Rust";
    let emb = deterministic_embedding(text);
    src_store
        .store_fact(
            "errors",
            text,
            "global",
            None,
            0.95,
            "user",
            &content_hash(text),
            &emb,
            "positive",
        )
        .unwrap();

    let bundle_path = export_bundle(&src_store, &src_config.home, None).unwrap();

    let dst_dir = TempDir::new().unwrap();
    let dst_config = test_config(&dst_dir);
    dst_config.ensure_dirs().unwrap();
    let dst_store = BrainStore::open(&dst_config.db_path).unwrap();
    let embedder = Embedder::deterministic();
    let report = import_bundle(
        &dst_store,
        &embedder,
        &bundle_path,
        MergePolicy::NewerWins,
        agent_brain::sync::SyncSource::ManualImport,
    )
    .unwrap();

    assert_eq!(report.imported, 1);
    let facts = dst_store.list_facts(10).unwrap();
    assert!(facts.iter().any(|f| f["topic"] == "errors"));
}

#[test]
fn user_source_memory_scores_higher_than_agent_source() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Always validate input at API boundaries";
    let emb = deterministic_embedding(text);

    store
        .store_fact_full(
            "validation-agent",
            text,
            "global",
            None,
            0.5,
            "agent",
            &content_hash(&format!("{text}-agent")),
            &emb,
            "positive",
            None,
        )
        .unwrap();
    store
        .store_fact_full(
            "validation-user",
            text,
            "global",
            None,
            0.5,
            "user",
            &content_hash(&format!("{text}-user")),
            &emb,
            "positive",
            None,
        )
        .unwrap();

    let query = "validate API input boundaries";
    let query_emb = deterministic_embedding(query);
    let (scored, _, _) = store
        .score_items(query, &query_emb, None, &[], false, None, None)
        .unwrap();

    let agent_score = scored
        .iter()
        .filter(|s| s.item_type == ItemType::Memory && s.topic == "validation-agent")
        .map(|s| s.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let user_score = scored
        .iter()
        .filter(|s| s.item_type == ItemType::Memory && s.topic == "validation-user")
        .map(|s| s.score)
        .fold(f64::NEG_INFINITY, f64::max);

    assert!(
        user_score > agent_score,
        "user source should beat agent (user={user_score}, agent={agent_score})"
    );
    assert!((user_score - agent_score - 0.08).abs() < 0.001);
}
