use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tempfile::TempDir;

use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::embed::deterministic_embedding;
use agent_brain::engine::Engine;
use agent_brain::settings::{AgentBrainSettings, UpstreamMcpSettings, UpstreamServerConfig};
use agent_brain::types::{ItemType, RouteLimits};
use agent_brain::upstream::IndexedUpstreamTool;

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
        precision_rerank_enabled: false,
        query_decompose_enabled: false,
        ann_enabled: true,
        ann_min_index: 1_500,
        ann_top_k: 100,
        workflow_dirs: vec![],
    }
}

fn seed_test_skills_agents_and_tools(store: &BrainStore) {
    let skills = [
        ("rust-testing", "Unit and integration testing with cargo test and proptest assertions"),
        ("db-migrations", "Database migrations schema evolution with diesel and sqlx"),
        ("docker-optimization", "Docker container multi-stage build optimization and caching"),
        ("kubernetes-deploy", "Kubernetes manifest deployment helm charts ingress configuration"),
        ("python-fastapi", "FastAPI asynchronous REST API endpoints pydantic models validation"),
    ];

    for (topic, text) in skills {
        let emb = deterministic_embedding(&format!("{topic} {text}"));
        store
            .upsert_indexed_item(
                ItemType::Skill,
                topic,
                text,
                &format!("/skills/{topic}/SKILL.md"),
                "global",
                None,
                &content_hash(text),
                Some(&emb),
            )
            .unwrap();
    }

    let agents = [
        ("research-agent", "Autonomous research agent exploring codebase searching documentation"),
        ("database-agent", "Database debugger executing explain plans analyzing query performance"),
    ];

    for (topic, text) in agents {
        let emb = deterministic_embedding(&format!("{topic} {text}"));
        store
            .upsert_indexed_item(
                ItemType::Agent,
                topic,
                text,
                &format!("/agents/{topic}.md"),
                "global",
                None,
                &content_hash(text),
                Some(&emb),
            )
            .unwrap();
    }

    let upstream_tools = vec![
        IndexedUpstreamTool {
            server: "devtools".into(),
            name: "run_shell_command".into(),
            description: "Run unit tests and cargo test commands in a sandboxed shell".into(),
        },
        IndexedUpstreamTool {
            server: "devtools".into(),
            name: "submit_workflow".into(),
            description: "Submit DAG workflow yaml to spine for multi-step autonomous execution".into(),
        },
        IndexedUpstreamTool {
            server: "devtools".into(),
            name: "inspect_dom".into(),
            description: "Capture and inspect web DOM tree screenshot accessibility elements".into(),
        },
        IndexedUpstreamTool {
            server: "devtools".into(),
            name: "budget_status".into(),
            description: "Check token budget quota status and remaining execution allocation".into(),
        },
    ];

    store.replace_upstream_tools(&upstream_tools).unwrap();
}

fn configure_upstream_settings(home: &std::path::Path) {
    let settings = AgentBrainSettings {
        upstream_mcp: UpstreamMcpSettings {
            enabled: true,
            suggest_limit: 4,
            servers: vec![
                UpstreamServerConfig {
                    name: "devtools".into(),
                    command: "devtools".into(),
                    args: vec!["serve".into()],
                    env: std::collections::HashMap::new(),
                    enabled: true,
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    };
    fs::write(
        home.join("config.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();
}

#[test]
fn agents_pick_expected_skills_and_mcp_tools() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    configure_upstream_settings(dir.path());
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    seed_test_skills_agents_and_tools(&store);
    let engine = Engine::new_with_store(config, store).unwrap();

    let test_cases = [
        (
            "run unit tests with cargo test",
            "rust-testing",
            "run_shell_command",
        ),
        (
            "execute multi-step workflow in spine",
            "docker-optimization",
            "submit_workflow",
        ),
        (
            "inspect webpage dom tree screenshot",
            "kubernetes-deploy",
            "inspect_dom",
        ),
    ];

    for (query, expected_skill_or_none, expected_tool) in test_cases {
        let resp = engine
            .route_task(
                query,
                None,
                &[],
                500,
                RouteLimits {
                    agents: 2,
                    skills: 3,
                    rules: 2,
                    memory: 2,
                },
                None,
                None,
            )
            .unwrap();

        assert!(
            resp.suggested_tools.iter().any(|t| t.tool == expected_tool),
            "Query {:?} expected MCP tool {}, got: {:?}",
            query,
            expected_tool,
            resp.suggested_tools
        );

        if query.contains("unit tests") {
            assert!(
                resp.recommended_skills.iter().any(|s| s.name == expected_skill_or_none),
                "Query {:?} expected skill {}, got: {:?}",
                query,
                expected_skill_or_none,
                resp.recommended_skills
            );
        }
    }
}

#[test]
fn concurrent_stress_test_routing_and_memory_storage() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    configure_upstream_settings(dir.path());
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    seed_test_skills_agents_and_tools(&store);
    let engine = Arc::new(Engine::new_with_store(config, store.clone()).unwrap());

    let num_threads = 8;
    let iterations_per_thread = 50;

    let queries = [
        "run unit tests with cargo test",
        "migrate postgresql database schema with migrations",
        "deploy kubernetes helm charts to staging",
        "execute shell command using bash",
        "thanks for the status update",
    ];

    let started = Instant::now();
    let mut handles = Vec::new();

    for thread_id in 0..num_threads {
        let eng = engine.clone();
        let st = store.clone();
        let handle = thread::spawn(move || {
            for i in 0..iterations_per_thread {
                let query = queries[(thread_id + i) % queries.len()];
                let resp = eng
                    .route_task(
                        query,
                        None,
                        &[],
                        500,
                        RouteLimits {
                            agents: 1,
                            skills: 2,
                            rules: 1,
                            memory: 2,
                        },
                        None,
                        None,
                    )
                    .unwrap();

                assert!(resp.must_apply.iter().any(|m| m.topic == "agstack-invariants"));

                if query.contains("unit tests") {
                    assert!(resp.recommended_skills.iter().any(|s| s.name == "rust-testing"));
                    assert!(resp.suggested_tools.iter().any(|t| t.tool == "run_shell_command"));
                } else if query.contains("thanks") {
                    assert!(resp.recommended_skills.is_empty());
                }

                if i % 10 == 0 {
                    let fact = format!("Thread {} established invariant milestone {}", thread_id, i);
                    let emb = deterministic_embedding(&fact);
                    let hash = content_hash(&fact);
                    st.store_fact(
                        &format!("thread-{}-milestone", thread_id),
                        &fact,
                        "global",
                        None,
                        1.0,
                        "agent",
                        &hash,
                        &emb,
                        "positive",
                    )
                    .unwrap();
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("thread panicked during stress test");
    }

    let elapsed = started.elapsed();
    let total_ops = num_threads * iterations_per_thread;
    let avg_op_ms = elapsed.as_millis() as f64 / total_ops as f64;

    assert!(
        avg_op_ms < 20.0,
        "Stress test average operation latency {}ms exceeded 20ms threshold",
        avg_op_ms
    );
}
