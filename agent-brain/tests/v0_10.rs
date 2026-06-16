use std::sync::Arc;

use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::embed::deterministic_embedding;
use agent_brain::engine::Engine;
use agent_brain::eval::{assert_ci_gate, run_ci_eval, RECALL_AT_3_THRESHOLD};
use agent_brain::memory_gc::run_memory_gc;
use agent_brain::operator_digest::weekly_digest;
use agent_brain::promote::{approve_staging, promote_fact_to_skill, reject_staging};
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

#[test]
fn promote_stages_skill_draft_pending_approval() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Always use barreled imports in this repo";
    let emb = deterministic_embedding(text);
    let stored = store
        .store_fact(
            "imports",
            text,
            "global",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "positive",
        )
        .unwrap();

    let result = promote_fact_to_skill(&store, &config.home, Some(&stored.id), None, None).unwrap();
    assert_eq!(result.status, "pending");
    assert!(result.draft_path.exists());

    let rows = store.list_skill_staging(Some("pending")).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, result.staging_id);
}

#[test]
fn promote_approve_writes_skill_file() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Prefer explicit error types over strings";
    let emb = deterministic_embedding(text);
    let stored = store
        .store_fact(
            "errors",
            text,
            "global",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "positive",
        )
        .unwrap();

    let staged = promote_fact_to_skill(&store, &config.home, Some(&stored.id), None, Some("error-patterns")).unwrap();
    let path = approve_staging(&store, &staged.staging_id).unwrap();
    assert!(path.exists());
    let body = std::fs::read_to_string(path).unwrap();
    assert!(body.contains("error-patterns"));
}

#[test]
fn promote_reject_marks_staging_rejected() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Temporary note";
    let emb = deterministic_embedding(text);
    let stored = store
        .store_fact(
            "temp",
            text,
            "global",
            None,
            0.5,
            "agent",
            &content_hash(text),
            &emb,
            "positive",
        )
        .unwrap();
    let staged = promote_fact_to_skill(&store, &config.home, Some(&stored.id), None, None).unwrap();
    reject_staging(&store, &staged.staging_id).unwrap();
    let row = store.get_skill_staging(&staged.staging_id).unwrap().unwrap();
    assert_eq!(row.status, "rejected");
}

#[test]
fn memory_gc_skips_negative_facts_without_force() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Never use Jest in this project";
    let emb = deterministic_embedding(text);
    let stored = store
        .store_fact(
            "testing",
            text,
            "global",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "negative",
        )
        .unwrap();
    store
        .record_context_feedback(&[stored.id.clone()], false)
        .unwrap();

    let report = run_memory_gc(&store, false, false).unwrap();
    assert!(report.skipped_protected >= 1 || report.ids.is_empty());
    assert!(store.get_fact(&stored.id).unwrap().is_some());
}

#[test]
fn memory_gc_archives_stale_session_digest() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Old session note about foo bar baz qux";
    let emb = deterministic_embedding(text);
    let stored = store
        .store_fact(
            "session-digest-cursor-old",
            text,
            "global",
            None,
            0.5,
            "session_digest",
            &content_hash(text),
            &emb,
            "positive",
        )
        .unwrap();

    let old = chrono::Utc::now().timestamp_millis() - 200_i64 * 24 * 3600 * 1000;
    store.set_fact_updated_at_for_test(&stored.id, old).unwrap();

    let dry = run_memory_gc(&store, true, false).unwrap();
    assert!(dry.ids.contains(&stored.id));

    let applied = run_memory_gc(&store, false, false).unwrap();
    assert_eq!(applied.archived, 1);
    assert!(store.get_fact(&stored.id).unwrap().is_none());
    assert!(
        applied
            .reason_buckets
            .iter()
            .any(|b| b.reason == "archive:stale_session_digest" && b.count >= 1),
        "{:?}",
        applied.reason_buckets
    );
    assert!(
        applied
            .top_topics
            .iter()
            .any(|t| t.topic == "session-digest-cursor-old"),
        "{:?}",
        applied.top_topics
    );
}

#[test]
fn memory_gc_report_buckets_protected_negative() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let text = "Never commit secrets to git";
    let emb = deterministic_embedding(text);
    let stored = store
        .store_fact(
            "secrets",
            text,
            "global",
            None,
            0.9,
            "agent",
            &content_hash(text),
            &emb,
            "negative",
        )
        .unwrap();
    store
        .record_context_feedback(&[stored.id.clone()], false)
        .unwrap();
    for _ in 0..8 {
        store
            .record_context_feedback(&[stored.id.clone()], false)
            .unwrap();
    }
    let old = chrono::Utc::now().timestamp_millis() - 100_i64 * 24 * 3600 * 1000;
    store
        .set_context_last_used_for_test(&stored.id, old)
        .unwrap();

    let report = run_memory_gc(&store, true, false).unwrap();
    assert!(
        report
            .reason_buckets
            .iter()
            .any(|b| b.reason == "protected:negative" && b.count >= 1),
        "{:?}",
        report.reason_buckets
    );
}

#[test]
fn weekly_digest_empty_db() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let digest = weekly_digest(&store, 7).unwrap();
    assert_eq!(digest.route_calls, 0);
}

#[test]
fn eval_ci_meets_recall_gate() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let engine = Engine::new_with_store(config, store).unwrap();
    let report = run_ci_eval(&engine).unwrap();
    assert!(report.recall_at_3 >= RECALL_AT_3_THRESHOLD, "{report:?}");
    assert_ci_gate(&report).unwrap();
}

#[test]
fn proofs_ci_passes_isolated_gates() {
    let report = agent_brain::proofs::run_ci_proofs().unwrap();
    agent_brain::proofs::assert_ci_proofs(&report).unwrap();
    assert!(report.eval.skills.recall_at_3 >= RECALL_AT_3_THRESHOLD);
    assert!(report.latency.passed);
}
