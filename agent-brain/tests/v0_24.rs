//! v0.24 — learning loop: trajectories, lineage, BEAM escalation suites.

use agent_brain::beam_eval::{assert_beam_gate, run_beam_eval_isolated};
use agent_brain::db::store::BrainStore;
use agent_brain::embed::Embedder;
use agent_brain::observation::{run_observations, ObservationConfig};
use agent_brain::trace_extract::{run_trace_extract, TraceExtractConfig};
use agent_brain::trajectory::store_trajectory;
use tempfile::TempDir;

#[test]
fn store_trajectory_links_route_log() {
    let dir = TempDir::new().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    store
        .insert_retrieval_log(
            "route-log-1",
            "hash",
            "verification",
            "[]",
            80,
            false,
            false,
            12,
            Some(100),
            Some(40),
            Some(0),
        )
        .unwrap();
    let report = store_trajectory(
        &store,
        "wf-demo",
        "verify-node",
        "success",
        Some("route-log-1"),
        Some("verification"),
        Some("cargo test passed"),
    )
    .unwrap();
    assert!(report.route_log_linked);
}

#[test]
fn observation_records_source_lineage() {
    let dir = TempDir::new().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    let embedder = Embedder::deterministic();
    let emb = vec![0.02; 384];
    for i in 0..3 {
        let fact = format!("Prefer Vitest for unit tests variant {i}");
        store
            .store_fact_full(
                "testing",
                &fact,
                "global",
                None,
                0.9,
                "agent",
                &agent_brain::db::store::content_hash(&fact),
                &emb,
                "positive",
                Some(r#"["phase:implementing"]"#),
                None,
            )
            .unwrap();
    }
    run_observations(
        &store,
        &embedder,
        &ObservationConfig::default(),
        false,
    )
    .unwrap();
    let facts = store.list_facts(10).unwrap();
    let obs = facts
        .iter()
        .find(|f| f["topic"] == "obs/testing")
        .expect("obs fact");
    let obs_id = obs["id"].as_str().unwrap();
    assert!(store.count_fact_lineage(obs_id).unwrap() >= 3);
}

#[test]
fn trace_extract_records_tool_log_lineage() {
    let dir = TempDir::new().unwrap();
    let home = dir.path().join(".agent_brain");
    std::fs::create_dir_all(&home).unwrap();
    let store = BrainStore::open(&home.join("brain.db")).unwrap();
    let embedder = Embedder::deterministic();
    store
        .insert_tool_log(
            "t1",
            "Shell",
            None,
            Some("cargo add anyhow"),
            0,
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
    run_trace_extract(
        &store,
        &embedder,
        &home,
        &TraceExtractConfig::default(),
        false,
    )
    .unwrap();
    let facts = store.list_facts(10).unwrap();
    let fact = facts
        .iter()
        .find(|f| f["topic"] == "deps-anyhow")
        .expect("trace fact");
    let fact_id = fact["id"].as_str().unwrap();
    assert_eq!(store.count_fact_lineage(fact_id).unwrap(), 1);
}

#[test]
fn beam_eval_includes_v024_suites() {
    let report = run_beam_eval_isolated().unwrap();
    assert_beam_gate(&report).unwrap();
    assert!(report.escalation_signal.cases >= 2);
    assert!(report.task_scoped_verification.cases >= 2);
}
