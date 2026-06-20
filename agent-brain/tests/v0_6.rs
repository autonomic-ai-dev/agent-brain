use std::fs;
use std::process::Command;

use sha2::Digest;

use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::embed::deterministic_embedding;
use agent_brain::settings::GitSyncSettings;
use agent_brain::sync::{git_clone, git_pull, git_push, init_git_repo};
use agent_brain::Engine;
use std::sync::Arc;
use tempfile::TempDir;

fn test_config(home: &std::path::Path) -> Config {
    Config {
        home: home.to_path_buf(),
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
fn git_push_pull_round_trip_via_bare_remote() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let workspace = TempDir::new().unwrap();
    let bare_path = workspace.path().join("bare.git");
    assert!(Command::new("git")
        .args(["init", "--bare", bare_path.to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    let remote = format!("file://{}", bare_path.display());
    let settings = GitSyncSettings {
        remote: remote.clone(),
        branch: "main".into(),
        ..Default::default()
    };

    let home_a = workspace.path().join("machine-a");
    fs::create_dir_all(&home_a).unwrap();
    init_git_repo(&home_a, Some(&remote), "main").unwrap();

    let config_a = test_config(&home_a);
    config_a.ensure_dirs().unwrap();
    let store_a = BrainStore::open(&config_a.db_path).unwrap();
    let text = "Memory synced from machine A via git bundle";
    let emb = deterministic_embedding(text);
    store_a
        .store_fact(
            "sync-test",
            text,
            "global",
            None,
            0.9,
            "user",
            &content_hash(text),
            &emb,
            "positive",
        )
        .unwrap();
    git_push(&store_a, &home_a, &settings).unwrap();

    let home_b = workspace.path().join("machine-b");
    fs::create_dir_all(&home_b).unwrap();
    git_clone(&home_b, &remote, "main").unwrap();

    let config_b = test_config(&home_b);
    config_b.ensure_dirs().unwrap();
    let store_b = Arc::new(BrainStore::open(&config_b.db_path).unwrap());
    let engine_b = Arc::new(Engine::new_with_store(config_b, store_b).unwrap());
    let report = git_pull(&engine_b, &settings).unwrap();

    assert_eq!(report.imported, 1);
    let facts = engine_b.store.list_facts(10).unwrap();
    assert!(facts.iter().any(|f| f["topic"] == "sync-test"));
}

#[test]
fn git_import_logs_conflicts_with_git_sync_source() {
    use agent_brain::embed::Embedder;
    use agent_brain::sync::{import_bundle, MergePolicy, SyncSource};

    let dir = TempDir::new().unwrap();
    let config = test_config(dir.path());
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = vec![0.02; 384];

    store
        .store_fact(
            "runner",
            "Use Jest",
            "global",
            None,
            0.9,
            "agent",
            &content_hash("Use Jest"),
            &emb,
            "positive",
        )
        .unwrap();

    let bundle_dir = dir.path().join("bundle");
    fs::create_dir_all(&bundle_dir).unwrap();
    let remote_fact = serde_json::json!({
        "id": "remote-1",
        "topic": "runner",
        "fact": "Use Vitest",
        "scope": "global",
        "scope_key": null,
        "source": "user",
        "confidence": 0.95,
        "polarity": "positive",
        "content_hash": content_hash("Use Vitest"),
        "created_at": chrono::Utc::now().timestamp_millis() + 1000,
        "updated_at": chrono::Utc::now().timestamp_millis() + 1000,
    });
    let facts_body = format!("{}\n", remote_fact);
    let checksum = format!("sha256:{:x}", sha2::Sha256::digest(facts_body.as_bytes()));
    fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::json!({
            "schema_version": 1,
            "device_id": "test-device",
            "exported_at": chrono::Utc::now().timestamp_millis(),
            "fact_count": 1,
            "includes_vectors": false,
            "checksums": { "facts_jsonl": checksum },
        })
        .to_string(),
    )
    .unwrap();
    fs::write(bundle_dir.join("facts.jsonl"), facts_body).unwrap();

    let embedder = Embedder::deterministic();
    import_bundle(
        &store,
        &embedder,
        &bundle_dir,
        MergePolicy::NewerWins,
        SyncSource::Git,
    )
    .unwrap();

    let conflicts = store.list_conflicts(5).unwrap();
    assert!(
        conflicts
            .iter()
            .any(|c| c["sync_source"] == "git" && c["topic"] == "runner"),
        "expected git-sourced conflict log entry"
    );
}

#[test]
fn sync_restore_repromotes_loser_fact() {
    use agent_brain::embed::Embedder;
    use agent_brain::sync::{import_bundle, restore_conflict, MergePolicy, SyncSource};

    let dir = TempDir::new().unwrap();
    let config = test_config(dir.path());
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = vec![0.02; 384];

    store
        .store_fact(
            "lint",
            "Use ESLint flat config",
            "global",
            None,
            0.9,
            "agent",
            &content_hash("Use ESLint flat config"),
            &emb,
            "positive",
        )
        .unwrap();

    let bundle_dir = dir.path().join("bundle");
    fs::create_dir_all(&bundle_dir).unwrap();
    let remote_fact = serde_json::json!({
        "id": "remote-lint",
        "topic": "lint",
        "fact": "Use Oxlint instead",
        "scope": "global",
        "scope_key": null,
        "source": "user",
        "confidence": 0.95,
        "polarity": "positive",
        "content_hash": content_hash("Use Oxlint instead"),
        "created_at": chrono::Utc::now().timestamp_millis() + 1000,
        "updated_at": chrono::Utc::now().timestamp_millis() + 1000,
    });
    let facts_body = format!("{}\n", remote_fact);
    let checksum = format!("sha256:{:x}", sha2::Sha256::digest(facts_body.as_bytes()));
    fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::json!({
            "schema_version": 1,
            "device_id": "test-device",
            "exported_at": chrono::Utc::now().timestamp_millis(),
            "fact_count": 1,
            "includes_vectors": false,
            "checksums": { "facts_jsonl": checksum },
        })
        .to_string(),
    )
    .unwrap();
    fs::write(bundle_dir.join("facts.jsonl"), facts_body).unwrap();

    let embedder = Embedder::deterministic();
    import_bundle(
        &store,
        &embedder,
        &bundle_dir,
        MergePolicy::NewerWins,
        SyncSource::Git,
    )
    .unwrap();

    let conflict_id = store
        .list_conflicts(1)
        .unwrap()
        .into_iter()
        .find(|c| c["topic"] == "lint")
        .and_then(|c| c["id"].as_str().map(String::from))
        .expect("conflict row");

    restore_conflict(&store, &embedder, &conflict_id).unwrap();

    let active = store
        .get_active_fact_by_topic("lint", "global", None)
        .unwrap()
        .expect("active lint fact");
    assert_eq!(active.fact, "Use ESLint flat config");

    let row = store
        .get_conflict(&conflict_id)
        .unwrap()
        .expect("conflict row");
    assert!(row.restored);
}
