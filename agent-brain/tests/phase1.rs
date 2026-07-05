use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_brain::cache::{
    fingerprint_open_files, fingerprint_query, route_cache_key, CacheKey, QueryEmbeddingCache,
    TurnCache,
};
use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, looks_like_secret, BrainStore};
use agent_brain::db::RouteLatencyStats;
use agent_brain::embed::{deterministic_embedding, Embedder};
use agent_brain::engine::Engine;
use agent_brain::tokens::estimate_tokens;
use agent_brain::types::{ItemType, RouteLimits, RouteTaskResponse};
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

fn dummy_embedding() -> Vec<f32> {
    vec![0.01; 384]
}

fn test_embedder() -> Arc<Embedder> {
    Arc::new(Embedder::deterministic())
}

#[test]
fn rejects_secret_patterns() {
    assert!(looks_like_secret("api_key=sk-abcdefghijklmnopqrstuvwxyz"));
    assert!(looks_like_secret("password: hunter2"));
    assert!(looks_like_secret("load from .env file"));
    assert!(!looks_like_secret("prefer Result types over unwrap"));
}

#[test]
fn deduplicates_identical_facts() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = dummy_embedding();
    let hash = content_hash("Use anyhow for errors");

    let first = store
        .store_fact(
            "errors",
            "Use anyhow for errors",
            "project",
            Some("/repo"),
            0.9,
            "agent",
            &hash,
            &emb,
            "positive",
        )
        .unwrap();
    assert!(first.stored);
    assert!(!first.deduplicated);

    let second = store
        .store_fact(
            "errors",
            "Use anyhow for errors",
            "project",
            Some("/repo"),
            0.9,
            "agent",
            &hash,
            &emb,
            "positive",
        )
        .unwrap();
    assert!(!second.stored);
    assert!(second.deduplicated);
    assert_eq!(first.id, second.id);
}

#[test]
fn add_only_same_topic_facts_preserve_history() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = dummy_embedding();

    let v1 = store
        .store_fact(
            "lint",
            "Run clippy on every PR",
            "project",
            Some("/repo"),
            0.9,
            "agent",
            &content_hash("Run clippy on every PR"),
            &emb,
            "positive",
        )
        .unwrap();
    let v2 = store
        .store_fact(
            "lint",
            "Run clippy and fmt on every PR",
            "project",
            Some("/repo"),
            0.9,
            "agent",
            &content_hash("Run clippy and fmt on every PR"),
            &emb,
            "positive",
        )
        .unwrap();

    assert_ne!(v1.id, v2.id);
    let active: Vec<_> = store.list_facts(10).unwrap();
    assert_eq!(active.len(), 2);
    let facts: Vec<_> = active.iter().map(|f| f["fact"].as_str().unwrap()).collect();
    assert!(facts.contains(&"Run clippy on every PR"));
    assert!(facts.contains(&"Run clippy and fmt on every PR"));
}

#[test]
fn turn_cache_ignores_open_files_when_configured() {
    let key_a = route_cache_key(
        "repo",
        "implement",
        "implementing",
        &["src/a.rs".into()],
        "fix bug",
        1,
        true,
    );
    let key_b = route_cache_key(
        "repo",
        "implement",
        "implementing",
        &["src/b.rs".into()],
        "fix bug",
        1,
        true,
    );
    assert_eq!(key_a.open_files_fp, key_b.open_files_fp);
    assert_eq!(key_a.query_fp, key_b.query_fp);

    let key_c = route_cache_key(
        "repo",
        "implement",
        "implementing",
        &["src/a.rs".into()],
        "fix bug",
        1,
        false,
    );
    assert_ne!(key_a.open_files_fp, key_c.open_files_fp);
}

#[test]
fn turn_cache_returns_hit_on_repeat_query() {
    let cache = TurnCache::new(8, 60);
    let key = CacheKey {
        scope_key: "repo".into(),
        phase: "implement".into(),
        open_files_fp: fingerprint_open_files(&["src/main.rs".into()]),
        query_fp: fingerprint_query("fix the routing bug"),
        index_version: 1,
        task_kind: "implementing".into(),
    };
    let resp = RouteTaskResponse {
        recommended_phase: "implement".into(),
        tokens_budget: 500,
        ..Default::default()
    };
    cache.put(key.clone(), resp);
    let hit = cache.get(&key).expect("cache hit");
    assert!(hit.cache_hit);
}

#[test]
fn truncates_context_to_token_budget() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = test_embedder();

    for i in 0..20 {
        let text =
            format!("Long rule number {i} with enough text to consume token budget quickly.");
        let emb = deterministic_embedding(&text);
        store
            .upsert_indexed_item(
                ItemType::Rule,
                &format!("rule-{i}"),
                &text,
                &format!("/rules/rule-{i}.md"),
                "global",
                None,
                &content_hash(&text),
                Some(&emb),
            )
            .unwrap();
    }

    let engine = Engine::new_with_store(config.clone(), Arc::new(store)).unwrap();

    let resp = engine
        .get_context("routing rules", None, 30, &[ItemType::Rule])
        .unwrap();

    assert!(resp.truncated || resp.items.len() < 20);
    assert!(resp.tokens_used <= resp.tokens_budget);
}

#[test]
fn route_task_respects_max_tokens() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = test_embedder();

    for i in 0..10 {
        let text = format!("Agent capability {i} for rust backend work");
        let emb = deterministic_embedding(&text);
        store
            .upsert_indexed_item(
                ItemType::Agent,
                &format!("agent-{i}"),
                &text,
                &format!("/agents/agent-{i}.md"),
                "global",
                None,
                &content_hash(&text),
                Some(&emb),
            )
            .unwrap();
    }

    let engine = Engine::new_with_store(config, Arc::new(store)).unwrap();

    let resp = engine
        .route_task(
            "implement rust backend",
            Some(PathBuf::from(".").as_path()),
            &[],
            20,
            RouteLimits {
                agents: 5,
                skills: 5,
                rules: 5,
                memory: 5,
            },
            None,
            None,
        )
        .unwrap();

    assert!(resp.tokens_used <= resp.tokens_budget);
}

#[test]
fn routes_skill_with_matching_description_over_unrelated_skill() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();

    let review_desc = "Use when reviewing pull request changes, diffs, and merge readiness";
    let cooking_desc = "Weeknight pasta recipes and baking tips for home cooks";
    let query = "let's review the changes on the PR";

    for (topic, text) in [("code-review", review_desc), ("cooking-tips", cooking_desc)] {
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

    let engine = Engine::new_with_store(config, Arc::new(store)).unwrap();
    let resp = engine
        .route_task(
            query,
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

    let top = resp
        .recommended_skills
        .first()
        .map(|s| s.name.as_str())
        .unwrap_or("");
    assert_eq!(
        top, "code-review",
        "expected description-matched skill, got {:?}",
        resp.recommended_skills
    );
}

#[test]
fn token_estimate_is_reasonable() {
    let text = "hello world";
    assert!(estimate_tokens(text) >= 2);
    assert!(estimate_tokens(text) < 20);
}

#[test]
fn dedupes_duplicate_skill_names_in_route_task() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = test_embedder();

    let query = "configure vitest for react testing";
    let strong = format!("{query} vitest react testing patterns");
    let weak = "unrelated cooking recipes and gardening tips";

    for (path, text) in [
        (
            "/packages/ecc/skills/react-patterns/SKILL.md",
            strong.as_str(),
        ),
        ("/packages/other/skills/react-patterns/SKILL.md", weak),
    ] {
        let emb = deterministic_embedding(text);
        store
            .upsert_indexed_item(
                ItemType::Skill,
                "react-patterns",
                text,
                path,
                "package",
                Some("pkg"),
                &content_hash(text),
                Some(&emb),
            )
            .unwrap();
    }

    let engine = Engine::new_with_store(config, Arc::new(store)).unwrap();

    let resp = engine
        .route_task(
            query,
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

    let skill_names: Vec<_> = resp
        .recommended_skills
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        skill_names
            .iter()
            .filter(|n| **n == "react-patterns")
            .count(),
        1
    );
    assert!(resp.recommended_skills[0]
        .path
        .contains("/packages/ecc/skills/react-patterns/"));
}

#[test]
fn query_embedding_cache_survives_reopen() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let emb = dummy_embedding();
    let hash = content_hash("persisted routing query");

    {
        let store = BrainStore::open(&config.db_path).unwrap();
        store.put_query_embedding(&hash, &emb).unwrap();
        let cached = store.get_query_embedding(&hash).unwrap().expect("cached");
        assert_eq!(cached.len(), emb.len());
    }

    let store = BrainStore::open(&config.db_path).unwrap();
    let cached = store
        .get_query_embedding(&hash)
        .unwrap()
        .expect("survives reopen");
    assert_eq!(cached.len(), emb.len());
}

#[test]
fn purges_indexed_items_under_package_prefix() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let emb = dummy_embedding();

    let prefix = dir.path().join("packages/ecc");
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "foo",
            "skill text",
            &prefix.join("skills/foo/SKILL.md").display().to_string(),
            "package",
            Some("ecc"),
            &content_hash("skill text"),
            Some(&emb),
        )
        .unwrap();
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "bar",
            "other skill",
            "/global/skills/bar/SKILL.md",
            "global",
            None,
            &content_hash("other skill"),
            Some(&emb),
        )
        .unwrap();

    let purged = store
        .delete_indexed_items_under_prefix(&prefix.display().to_string())
        .unwrap();
    assert_eq!(purged, 1);

    let remaining = store.load_searchable_items().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].topic, "bar");
}

#[test]
fn zero_route_limits_normalize_to_defaults() {
    let limits = RouteLimits {
        agents: 0,
        skills: 0,
        rules: 0,
        memory: 0,
    }
    .normalize();
    assert_eq!(limits.agents, 2);
    assert_eq!(limits.skills, 3);
    assert_eq!(limits.rules, 5);
    assert_eq!(limits.memory, 5);
}

#[test]
fn partial_zero_route_limits_are_preserved() {
    let limits = RouteLimits {
        agents: 0,
        skills: 3,
        rules: 0,
        memory: 0,
    }
    .normalize();
    assert_eq!(limits.skills, 3);
    assert_eq!(limits.agents, 0);
}

#[test]
fn route_task_with_all_zero_limits_returns_skills() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    let embedder = test_embedder();

    let text = "spring boot junit mockmvc test patterns";
    let emb = deterministic_embedding(text);
    store
        .upsert_indexed_item(
            ItemType::Skill,
            "springboot-tdd",
            text,
            "/skills/springboot-tdd/SKILL.md",
            "package",
            Some("ecc"),
            &content_hash(text),
            Some(&emb),
        )
        .unwrap();

    let engine = Engine::new_with_store(config, Arc::new(store)).unwrap();

    let resp = engine
        .route_task(
            "write spring boot integration tests with mockmvc",
            None,
            &[],
            500,
            RouteLimits {
                agents: 0,
                skills: 0,
                rules: 0,
                memory: 0,
            },
            None,
            None,
        )
        .unwrap();

    assert!(
        !resp.recommended_skills.is_empty(),
        "expected skills after zero-limit normalization"
    );
    assert!(resp.tokens_used > 0);
}

#[test]
fn route_limits_schema_defaults_are_nonzero() {
    let schema = schemars::schema_for!(RouteLimits);
    let json = serde_json::to_value(&schema).unwrap();
    let props = json
        .pointer("/properties")
        .and_then(|v| v.as_object())
        .expect("properties");
    for field in ["agents", "skills", "rules", "memory"] {
        let default = props
            .get(field)
            .and_then(|v| v.get("default"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert!(default > 0, "{field} schema default should be > 0");
    }
}
