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

    let staged = promote_fact_to_skill(
        &store,
        &config.home,
        Some(&stored.id),
        None,
        Some("error-patterns"),
    )
    .unwrap();
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
    let row = store
        .get_skill_staging(&staged.staging_id)
        .unwrap()
        .unwrap();
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
fn skills_sh_eval_passes_on_committed_fixture_db() {
    let fixture = agent_brain::fixture::default_fixture_2k_path();
    assert!(
        fixture.exists(),
        "missing {} — run: cargo run -p agent-brain -- fixture build",
        fixture.display()
    );
    let snapshot = agent_brain::skills_sh::default_snapshot_path();
    let golden = agent_brain::skills_sh::default_golden_path();
    let report =
        agent_brain::skills_sh::run_skills_sh_eval(&snapshot, &golden, Some(&fixture)).unwrap();
    agent_brain::skills_sh::assert_skills_sh_gate(&report).unwrap();
    assert_eq!(report.index_mode, "fixture-db");
    assert_eq!(
        report.simulated_index_size,
        agent_brain::skills_sh::SKILLS_SH_SIMULATED_INDEX
    );
    assert!(report.snapshot_skills >= 3);
    assert_eq!(
        report.filler_skills,
        report
            .simulated_index_size
            .saturating_sub(report.snapshot_skills)
    );
}

#[test]
fn fixture_2k_db_has_expected_composition() {
    let fixture = agent_brain::fixture::default_fixture_2k_path();
    let meta = agent_brain::fixture::read_fixture_meta(&fixture).unwrap();
    assert_eq!(meta.index_size, 2000);
    let dir = tempfile::tempdir().unwrap();
    let config = agent_brain::config::Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs().unwrap();
    std::fs::copy(&fixture, &config.db_path).unwrap();
    let store = agent_brain::db::store::BrainStore::open(&config.db_path).unwrap();
    let breakdown = agent_brain::fixture::fixture_db_breakdown(&store).unwrap();
    assert_eq!(breakdown.total_indexed, 2000);
    assert_eq!(breakdown.skills_sh_rows + breakdown.bench_filler_rows, 2000);
    // Real-only fixture (recipe v2): no bench fillers when snapshot has 2000 skills
    if meta.recipe_version == "2" && meta.snapshot_skills >= 2000 {
        assert_eq!(breakdown.bench_filler_rows, 0);
        assert_eq!(breakdown.skills_sh_rows, 2000);
        assert_eq!(meta.filler_skills, 0);
    }
}

#[test]
fn skills_sh_eval_passes_runtime_seed() {
    let snapshot = agent_brain::skills_sh::default_snapshot_path();
    let golden = agent_brain::skills_sh::default_golden_path();
    let report = agent_brain::skills_sh::run_skills_sh_eval(&snapshot, &golden, None).unwrap();
    agent_brain::skills_sh::assert_skills_sh_gate(&report).unwrap();
    assert_eq!(report.index_mode, "runtime-seed");
}

#[test]
fn proofs_ci_passes_isolated_gates() {
    let report = agent_brain::proofs::run_ci_proofs().unwrap();
    agent_brain::proofs::assert_ci_proofs(&report).unwrap();
    assert!(report.eval.skills.recall_at_3 >= RECALL_AT_3_THRESHOLD);
    assert!(report.latency.passed);
    assert!(report.supervisor.passed);
    assert!(report.token_tools.passed);
    assert!(report.scale.passed);
    assert!(report.eval.cases >= 20);
}

#[test]
fn stats_collects_token_savings_from_retrieval_log() {
    use agent_brain::stats;

    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    store
        .insert_retrieval_log(
            "log-1",
            "abc",
            "implementing",
            "[]",
            120,
            false,
            false,
            15,
            Some(2000),
            Some(95),
            Some(2),
        )
        .unwrap();

    let snapshot = stats::collect(&store, &config, 7).unwrap();
    assert_eq!(snapshot.period.route_calls, 1);
    assert_eq!(snapshot.period.routes_with_constraints, 1);
    assert_eq!(snapshot.period.total_must_apply, 2);
    assert_eq!(snapshot.period.routes_with_savings, 1);
    assert!(snapshot.period.avg_saved_pct >= 95.0);
    assert!(snapshot.period.total_saved_tokens > 0);
    assert!(stats::format_summary_line(&snapshot).contains("tok saved"));
}

#[test]
fn adoption_milestones_record_first_route_once() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    agent_brain::adoption::ensure_installed_at(&store).unwrap();
    assert!(store
        .get_meta(agent_brain::adoption::INSTALLED_AT)
        .unwrap()
        .is_some());
    agent_brain::adoption::record_first_route(&store).unwrap();
    let first = store
        .get_meta(agent_brain::adoption::FIRST_ROUTE_AT)
        .unwrap()
        .clone();
    agent_brain::adoption::record_first_route(&store).unwrap();
    assert_eq!(
        store
            .get_meta(agent_brain::adoption::FIRST_ROUTE_AT)
            .unwrap(),
        first
    );
}

#[test]
fn supervisor_bundle_routes_token_efficient_skill() {
    use agent_brain::packages::install_bundled;
    use agent_brain::types::RouteLimits;

    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    install_bundled(&config, "supervisor").unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let engine = Arc::new(Engine::new_with_store(config, Arc::clone(&store)).unwrap());
    let n = engine.bootstrap(None).unwrap();
    assert!(n >= 3, "expected supervisor skills/rules indexed, got {n}");

    let resp = engine
        .route_task(
            "grep before cat a large log file token efficient",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 3,
                rules: 2,
                memory: 0,
            },
            Some("implementing"),
            None,
        )
        .unwrap();
    let skill_names: Vec<_> = resp
        .recommended_skills
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        skill_names.iter().any(|n| *n == "token-efficient-ops"),
        "expected token-efficient-ops in {:?}",
        skill_names
    );
}

#[test]
fn must_apply_promoted_without_memory_budget() {
    use agent_brain::db::store::{content_hash, BrainStore};
    use agent_brain::embed::deterministic_embedding;
    use agent_brain::packages::install_bundled;
    use agent_brain::types::RouteLimits;

    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    install_bundled(&config, "supervisor").unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let fact = "Never read dist/ or build output directories with cat or read_file";
    let emb = deterministic_embedding(fact);
    store
        .store_fact(
            "no-read-dist",
            fact,
            "global",
            None,
            0.95,
            "eval",
            &content_hash(fact),
            &emb,
            "negative",
        )
        .unwrap();
    store.bump_index_version().unwrap();
    let engine = Arc::new(Engine::new_with_store(config, Arc::clone(&store)).unwrap());
    engine.bootstrap(None).unwrap();

    let resp = engine
        .route_task(
            "read frontend build artifacts from dist folder deployment",
            None,
            &[],
            10,
            RouteLimits {
                agents: 0,
                skills: 0,
                rules: 0,
                memory: 1,
            },
            Some("implementing"),
            None,
        )
        .unwrap();
    assert!(
        resp.must_apply.iter().any(|m| m.topic == "no-read-dist"),
        "expected no-read-dist in must_apply, got {:?}",
        resp.must_apply
    );
}

#[test]
fn route_task_suggests_native_token_tools_for_file_queries() {
    use agent_brain::packages::install_bundled;
    use agent_brain::types::RouteLimits;

    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    install_bundled(&config, "supervisor").unwrap();
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let engine = Arc::new(Engine::new_with_store(config, Arc::clone(&store)).unwrap());
    engine.bootstrap(None).unwrap();

    let resp = engine
        .route_task(
            "grep before cat a large log file for errors",
            None,
            &[],
            500,
            RouteLimits::default(),
            Some("debugging"),
            None,
        )
        .unwrap();
    assert!(
        resp.suggested_native_tools
            .iter()
            .any(|t| t.tool == "grep_search"),
        "expected grep_search suggestion, got {:?}",
        resp.suggested_native_tools
    );
}

#[test]
fn suggest_memory_approve_stores_negative_with_apply_when() {
    use std::fs;

    let dir = tempfile::TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let hooks = config.home.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let state = serde_json::json!({
        "anti_pattern_suggestion": {
            "topic": "no-read-dist",
            "fact": "Never read /app/dist/bundle.js whole — use grep_search.",
            "polarity": "negative",
            "path": "/app/dist/bundle.js",
            "reason": "blocked path"
        }
    });
    fs::write(
        hooks.join("route_state.json"),
        serde_json::to_string(&state).unwrap(),
    )
    .unwrap();

    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let engine = Engine::new_with_store(config, store).unwrap();
    let report = agent_brain::suggest_memory::approve_pending(&engine).unwrap();
    assert!(report.stored);
    assert_eq!(report.polarity, "negative");
    assert_eq!(report.apply_when, vec!["path:**/dist/**"]);
    assert!(
        agent_brain::route_briefing::read_anti_pattern_suggestion(&engine.config.home).is_none()
    );
}
