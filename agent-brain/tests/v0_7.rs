use std::fs;

use sha2::Digest;

use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::embed::deterministic_embedding;
use agent_brain::settings::CloudSyncSettings;
use agent_brain::sync::{cloud_pull, cloud_push};
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

fn local_cloud_settings(bucket: &std::path::Path) -> CloudSyncSettings {
    CloudSyncSettings {
        enabled: true,
        provider: "local".into(),
        bucket: bucket.display().to_string(),
        key: "brain-sync.tar.zst.age".into(),
        encrypt: true,
        encryption_key_env: "AGENT_BRAIN_SYNC_KEY".into(),
        ..Default::default()
    }
}

#[test]
fn cloud_push_pull_round_trip_local_provider() {
    std::env::set_var(
        "AGENT_BRAIN_SYNC_KEY",
        "test-sync-passphrase-32-chars-min!!",
    );

    let workspace = TempDir::new().unwrap();
    let cloud_bucket = workspace.path().join("cloud");
    fs::create_dir_all(&cloud_bucket).unwrap();
    let settings = local_cloud_settings(&cloud_bucket);

    let home_a = workspace.path().join("machine-a");
    fs::create_dir_all(&home_a).unwrap();
    let config_a = test_config(&home_a);
    config_a.ensure_dirs().unwrap();
    let store_a = BrainStore::open(&config_a.db_path).unwrap();
    let text = "Memory synced from machine A via encrypted cloud blob";
    let emb = deterministic_embedding(text);
    store_a
        .store_fact(
            "cloud-sync-test",
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
    cloud_push(&store_a, &home_a, &settings).unwrap();

    let home_b = workspace.path().join("machine-b");
    fs::create_dir_all(&home_b).unwrap();
    let config_b = test_config(&home_b);
    config_b.ensure_dirs().unwrap();
    let store_b = Arc::new(BrainStore::open(&config_b.db_path).unwrap());
    let engine_b = Arc::new(Engine::new_with_store(config_b, store_b).unwrap());
    let report = cloud_pull(&engine_b, &settings).unwrap();

    assert_eq!(report.import.imported, 1);
    let facts = engine_b.store.list_facts(10).unwrap();
    assert!(facts.iter().any(|f| f["topic"] == "cloud-sync-test"));

    std::env::remove_var("AGENT_BRAIN_SYNC_KEY");
}

#[test]
fn cloud_import_logs_conflicts_with_cloud_sync_source() {
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
    fs::write(bundle_dir.join("secret_refs.json"), "[]").unwrap();

    let embedder = Embedder::deterministic();
    import_bundle(
        &store,
        &embedder,
        &bundle_dir,
        MergePolicy::NewerWins,
        SyncSource::Cloud,
    )
    .unwrap();

    let conflicts = store.list_conflicts(5).unwrap();
    assert!(
        conflicts
            .iter()
            .any(|c| c["sync_source"] == "cloud" && c["topic"] == "runner"),
        "expected cloud-sourced conflict log entry"
    );
}

#[test]
fn secret_refs_merge_and_status() {
    let dir = TempDir::new().unwrap();
    let config = test_config(dir.path());
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();

    store
        .merge_secret_refs(&[agent_brain::secrets::SecretRef {
            name: "GITHUB_TOKEN".into(),
            used_by: "upstream_mcp.github".into(),
        }])
        .unwrap();

    std::env::set_var("GITHUB_TOKEN", "ghp_test");
    let status = agent_brain::secrets::secrets_status(&store).unwrap();
    assert!(status.configured.contains(&"GITHUB_TOKEN".into()));
    std::env::remove_var("GITHUB_TOKEN");

    let status = agent_brain::secrets::secrets_status(&store).unwrap();
    assert!(status.missing.contains(&"GITHUB_TOKEN".into()));
}
