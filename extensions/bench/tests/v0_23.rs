//! v0.23 — gRPC bridge, BEAM eval harness, and trace extraction.

#[cfg(feature = "extras")]
use agent_brain_bench::beam_eval::{assert_beam_gate, run_beam_eval_isolated};
use agent_brain::db::store::BrainStore;
use agent_brain::embed::Embedder;
use agent_brain::trace_extract::{run_trace_extract, TraceExtractConfig};
use tempfile::TempDir;

#[cfg(feature = "extras")]
#[test]
fn beam_eval_passes_isolated_harness() {
    let report = run_beam_eval_isolated().unwrap();
    assert_beam_gate(&report).unwrap();
    assert!(report.overall_score >= 0.85);
}

#[test]
fn trace_extract_creates_fact_from_shell_trace() {
    let dir = TempDir::new().unwrap();
    let home = dir.path().join(".agent_brain");
    std::fs::create_dir_all(&home).unwrap();
    let db = home.join("brain.db");
    let store = BrainStore::open(&db).unwrap();
    let embedder = Embedder::deterministic();

    store
        .insert_tool_log(
            "t1",
            "Shell",
            None,
            Some("npm install -D vitest"),
            0,
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();

    let report = run_trace_extract(
        &store,
        &embedder,
        &home,
        &TraceExtractConfig::default(),
        false,
    )
    .unwrap();
    assert_eq!(report.extracted, 1);

    let facts = store.list_facts(10).unwrap();
    assert!(facts.iter().any(|f| f["topic"] == "deps-vitest"));
}

