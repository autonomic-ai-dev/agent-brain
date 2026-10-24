use std::sync::Arc;
use tempfile::TempDir;

use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::embed::deterministic_embedding;
use agent_brain::engine::Engine;
use agent_brain::types::{ItemType, RouteLimits};

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

#[test]
fn agstack_mandatory_invariants_always_present_in_must_apply() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());
    let engine = Engine::new_with_store(config, store).unwrap();

    let queries = [
        "implement feature x",
        "why is git diff failing?",
        "thanks!",
        "write unit tests",
    ];

    for q in queries {
        let resp = engine
            .route_task(
                q,
                None,
                &[],
                500,
                RouteLimits {
                    agents: 1,
                    skills: 3,
                    rules: 2,
                    memory: 2,
                },
                None,
                None,
            )
            .unwrap();

        let invariant = resp
            .must_apply
            .iter()
            .find(|m| m.topic == "agstack-invariants");
        assert!(
            invariant.is_some(),
            "agstack-invariants must be present in must_apply for query: {}",
            q
        );
        let text = &invariant.unwrap().text;
        assert!(text.contains("Immutability first"));
        assert!(text.contains("Zero AI comment slop"));
        assert!(text.contains("Boundary discipline"));
        assert!(text.contains("Fix root causes"));
    }
}

#[test]
fn agstack_skill_applied_as_fallback_when_no_domain_skill_matches() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());

    let agstack_text = "Universal engineering operating system for autonomous coding agents enforcing immutability and verification harness";
    let agstack_emb = deterministic_embedding(&format!("agstack {}", agstack_text));
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "agstack",
            agstack_text,
            "/packages/agstack/SKILL.md",
            "global",
            None,
            &content_hash(agstack_text),
            Some(&agstack_emb),
        )
        .unwrap();

    let rust_text = "Unit testing with cargo test and proptest assertions";
    let rust_emb = deterministic_embedding(&format!("rust-testing {}", rust_text));
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "rust-testing",
            rust_text,
            "/packages/rust-testing/SKILL.md",
            "global",
            None,
            &content_hash(rust_text),
            Some(&rust_emb),
        )
        .unwrap();

    let engine = Engine::new_with_store(config, store).unwrap();

    let resp = engine
        .route_task(
            "implement generic state transition pipeline in the backend service",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 3,
                rules: 0,
                memory: 0,
            },
            None,
            None,
        )
        .unwrap();

    assert!(
        resp.recommended_skills.iter().any(|s| s.name == "agstack"),
        "agstack must be recommended as universal fallback, got: {:?}",
        resp.recommended_skills
    );
}

#[test]
fn agstack_skill_yields_to_specialized_domain_skill() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());

    let agstack_text = "Universal engineering operating system for autonomous coding agents enforcing immutability and verification harness";
    let agstack_emb = deterministic_embedding(&format!("agstack {}", agstack_text));
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "agstack",
            agstack_text,
            "/packages/agstack/SKILL.md",
            "global",
            None,
            &content_hash(agstack_text),
            Some(&agstack_emb),
        )
        .unwrap();

    let rust_text = "Unit testing with cargo test and proptest assertions";
    let rust_emb = deterministic_embedding(&format!("rust-testing {}", rust_text));
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "rust-testing",
            rust_text,
            "/packages/rust-testing/SKILL.md",
            "global",
            None,
            &content_hash(rust_text),
            Some(&rust_emb),
        )
        .unwrap();

    let engine = Engine::new_with_store(config, store).unwrap();

    let resp = engine
        .route_task(
            "write unit tests with cargo test",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 3,
                rules: 0,
                memory: 0,
            },
            None,
            None,
        )
        .unwrap();

    assert_eq!(
        resp.recommended_skills.first().map(|s| s.name.as_str()),
        Some("rust-testing"),
        "specialized skill rust-testing must be the top recommendation, got: {:?}",
        resp.recommended_skills
    );
}

#[test]
fn conversational_turns_suppress_agstack_skill_while_preserving_invariants() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());

    let agstack_text = "Universal engineering operating system for autonomous coding agents";
    let agstack_emb = deterministic_embedding(&format!("agstack {}", agstack_text));
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "agstack",
            agstack_text,
            "/packages/agstack/SKILL.md",
            "global",
            None,
            &content_hash(agstack_text),
            Some(&agstack_emb),
        )
        .unwrap();

    let engine = Engine::new_with_store(config, store).unwrap();

    let resp = engine
        .route_task(
            "thanks, got it! sounds good.",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 3,
                rules: 0,
                memory: 0,
            },
            None,
            None,
        )
        .unwrap();

    assert!(
        resp.recommended_skills.is_empty(),
        "conversational turn must not recommend procedural skills even when agstack is installed, got: {:?}",
        resp.recommended_skills
    );
    assert!(
        resp.must_apply
            .iter()
            .any(|m| m.topic == "agstack-invariants"),
        "agstack-invariants must still be present in must_apply during conversational turns"
    );
}

#[test]
fn agstack_project_rule_is_applied_to_applicable_rules() {
    let dir = TempDir::new().unwrap();
    let ws_dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = Arc::new(BrainStore::open(&config.db_path).unwrap());

    let rule_text = "Engineering Policy: immutability, zero comment slop, boundary validation";
    let rule_emb = deterministic_embedding(&format!("agstack-policy {}", rule_text));
    store
        .upsert_indexed_item(
            ItemType::Rule,
            "agstack-policy",
            rule_text,
            &format!("{}/.cursor/rules/agstack.mdc", ws_dir.path().display()),
            "project",
            Some(&ws_dir.path().display().to_string()),
            &content_hash(rule_text),
            Some(&rule_emb),
        )
        .unwrap();

    let engine = Engine::new_with_store(config, store).unwrap();

    let resp = engine
        .route_task(
            "refactor payment service handler",
            Some(ws_dir.path()),
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 0,
                rules: 3,
                memory: 0,
            },
            None,
            None,
        )
        .unwrap();

    assert!(
        resp.applicable_rules
            .iter()
            .any(|r| r.topic == "agstack-policy"),
        "agstack project rule must be present in applicable_rules, got: {:?}",
        resp.applicable_rules
    );
}
