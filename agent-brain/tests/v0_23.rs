//! v0.23 — gRPC bridge, BEAM eval harness, and trace extraction.

use agent_brain::beam_eval::{assert_beam_gate, run_beam_eval_isolated};
use agent_brain::db::store::BrainStore;
use agent_brain::embed::Embedder;
use agent_brain::grpc::convert::{route_response_to_proto, task_kind_to_proto};
use agent_brain::trace_extract::{run_trace_extract, TraceExtractConfig};
use agent_brain::types::{RouteTaskResponse, TaskKind};
use tempfile::TempDir;

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

#[test]
fn grpc_route_response_includes_bridge_fields() {
    let mut resp = RouteTaskResponse {
        task_kind: Some("verification".into()),
        route_confidence: 0.82,
        escalate_recommended: false,
        ..Default::default()
    };
    let pb = route_response_to_proto(resp.clone());
    assert_eq!(pb.route_confidence, 0.82);
    assert_eq!(pb.task_kind, task_kind_to_proto(TaskKind::Verification));
    resp.route_confidence = 0.1;
    resp.escalate_recommended = true;
    let pb = route_response_to_proto(resp);
    assert!(pb.escalate_recommended);
}
