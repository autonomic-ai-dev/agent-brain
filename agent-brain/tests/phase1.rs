use std::path::PathBuf;
use std::sync::Arc;

use agent_brain::cache::{
    fingerprint_open_files, fingerprint_query, route_cache_key, CacheKey, TurnCache,
};
use agent_brain::config::Config;
use agent_brain::db::store::{content_hash, looks_like_secret, BrainStore};
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
        precision_rerank_enabled: false,
        query_decompose_enabled: false,
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
    let _embedder = test_embedder();

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
    let _embedder = test_embedder();

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
    let _embedder = test_embedder();

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
    let _embedder = test_embedder();

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

#[test]
fn unrelated_query_in_tagged_workspace_yields_zero_skills() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = BrainStore::open(&config.db_path).unwrap();
    let ws_dir = TempDir::new().unwrap();
    std::fs::write(
        ws_dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .unwrap();

    let skills = [
        (
            "rust-patterns",
            "Idiomatic Rust patterns, ownership, error handling, traits, and best practices",
        ),
        (
            "rust-testing",
            "Rust unit and integration testing with cargo test, proptest, and mockall",
        ),
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

    let engine = Engine::new_with_store(config, Arc::new(store)).unwrap();

    let unrelated = engine
        .route_task(
            "currently getting many false positive skill suggestions, need to rethink how the suggestion is working.",
            Some(ws_dir.path()),
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
        unrelated.recommended_skills.is_empty(),
        "expected 0 skills for unrelated query in Rust workspace, got: {:?}",
        unrelated.recommended_skills
    );

    let related = engine
        .route_task(
            "write unit tests with cargo test",
            Some(ws_dir.path()),
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
        related
            .recommended_skills
            .iter()
            .any(|s| s.name == "rust-testing"),
        "expected rust-testing for matching query, got: {:?}",
        related.recommended_skills
    );
}

#[test]
fn large_scale_isolated_routing_benchmark_intent_vs_context() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let store = BrainStore::open(&config.db_path).unwrap();
    let ws_dir = TempDir::new().unwrap();
    std::fs::write(
        ws_dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo-service\"\n",
    )
    .unwrap();

    let skills: &[(&str, &str)] = &[
        ("rust-patterns", "Idiomatic Rust patterns, ownership, borrow checker, traits, concurrency, and error handling with Result"),
        ("rust-testing", "Rust unit and integration testing with cargo test, proptest, and mockall assertions"),
        ("rust-async", "Asynchronous Rust programming with tokio runtime, futures, channels, and select"),
        ("rust-wasm", "WebAssembly generation using wasm-bindgen, wasm-pack, and web-sys in Rust"),
        ("rust-embedded", "Embedded bare-metal Rust programming with no_std, cortex-m, and volatile registers"),
        ("rust-serde", "Data serialization and deserialization with serde_json, bincode, and custom visitors"),
        ("rust-ffi", "Foreign function interface C ABI bindings, libc, and cbindgen in Rust"),
        ("rust-performance", "Profiling, flamegraphs, criterion benchmarks, and memory allocation in Rust"),
        ("python-asyncio", "Python asyncio coroutines, event loop, tasks, and concurrent execution"),
        ("python-fastapi", "FastAPI web framework, Pydantic request models, dependencies, and OpenAPI endpoints"),
        ("python-django", "Django ORM models, migrations, views, and admin site management"),
        ("python-pytest", "Pytest unit testing fixtures, parametrize, and mock monkeypatching"),
        ("python-numpy", "NumPy array vectorization, broadcasting, linear algebra, and numerical computing"),
        ("python-pandas", "Pandas DataFrame manipulation, CSV parsing, aggregations, and data analysis"),
        ("python-typing", "Type hinting with mypy, typing Protocol, Generics, and runtime inspection"),
        ("python-packaging", "Building Python wheels with pyproject.toml, hatchling, poetry, and setuptools"),
        ("typescript-types", "TypeScript strict null checks, generics, conditional types, and type guards"),
        ("typescript-eslint", "ESLint rules, parser configuration, and stylistic code conventions in TypeScript"),
        ("react-streaming", "React server components streaming suspense boundaries and data fetching"),
        ("react-hooks", "Custom React hooks, useEffect lifecycle, useMemo, and useCallback optimization"),
        ("nextjs-routing", "Next.js app router server actions layouts middleware and static generation"),
        ("vue-composition", "Vue 3 composition API reactive state ref computed watch and component props"),
        ("angular-services", "Angular dependency injection services observables RxJS pipes and modules"),
        ("svelte-stores", "Svelte stores reactive declarations component bindings and animations"),
        ("jest-testing", "Jest snapshot testing mock functions expect assertions and coverage report"),
        ("vitest-testing", "Vitest unit testing in Vite projects mock timers and coverage v8"),
        ("cypress-e2e", "Cypress end-to-end browser automation page objects and network mocking"),
        ("playwright-automation", "Playwright cross-browser testing headless chrome traces and locators"),
        ("go-concurrency", "Golang goroutines channel synchronization mutex and select patterns"),
        ("go-testing", "Go test package table-driven tests subtests and benchmark profiling"),
        ("go-http", "Building Go net/http REST API services middleware routers and JSON handling"),
        ("go-gorm", "GORM object-relational mapping database queries associations and transactions"),
        ("go-grpc", "gRPC protobuf service definitions client stubs and streaming RPCs in Go"),
        ("java-spring", "Spring Boot microservices dependency injection autowired and application properties"),
        ("java-concurrency", "Java virtual threads executors CompletableFuture and memory model"),
        ("java-maven", "Maven pom.xml dependency management plugins phases and lifecycle goals"),
        ("java-gradle", "Gradle build.gradle Kotlin DSL tasks dependencies and multi-project builds"),
        ("kotlin-coroutines", "Kotlin coroutines dispatchers flow suspend functions and structured concurrency"),
        ("csharp-dotnet", "C# .NET async await LINQ nullable reference types and entity framework"),
        ("cpp-raii", "Modern C++ RAII unique_ptr shared_ptr move semantics and smart pointers"),
        ("cpp-templates", "C++ template metaprogramming concepts constraints SFINAE and type traits"),
        ("cpp-cmake", "CMakeLists.txt targets find_package target_link_libraries and compiler flags"),
        ("docker-optimization", "Docker multi-stage build layer caching slim images and dockerignore"),
        ("docker-compose", "Docker compose services networking volumes and environment configuration"),
        ("kubernetes-deploy", "Kubernetes helm chart deployment ingress configuration services and pods"),
        ("kubernetes-operator", "Kubernetes custom resource definitions CRD and controller reconciliation"),
        ("terraform-aws", "AWS terraform infrastructure as code setup VPC EC2 and security groups"),
        ("terraform-modules", "Reusable Terraform modules variables outputs state backend and locks"),
        ("ansible-playbooks", "Ansible playbooks roles inventory tasks handlers and YAML templates"),
        ("nginx-reverse-proxy", "Nginx reverse proxy load balancing SSL TLS termination and cache rules"),
        ("linux-systemd", "Linux systemd service unit configuration journalctl logging and daemon reload"),
        ("linux-bash", "Bash shell scripting piping exit codes signal traps and text processing"),
        ("postgres-pooling", "Postgres connection pooling with pgbouncer transaction mode and connections"),
        ("postgres-indexing", "PostgreSQL B-Tree GIN indexes EXPLAIN ANALYZE query planning and vacuum"),
        ("mysql-replication", "MySQL primary replica replication binlog innodb transactions and locks"),
        ("sqlite-wal", "SQLite WAL mode PRAGMA synchronous foreign keys and busy timeout settings"),
        ("mongodb-aggregations", "MongoDB aggregation pipelines match group lookup unwind and indexes"),
        ("redis-caching", "Redis pubsub caching eviction policy string keys and hash sets"),
        ("kafka-event-streaming", "Apache Kafka topics partitions consumer groups offsets and event streaming"),
        ("rabbitmq-queues", "RabbitMQ message queues exchanges routing keys and dead-letter queues"),
        ("nats-jetstream", "NATS JetStream pubsub key-value stores streams consumers and acknowledgments"),
        ("graphql-federation", "GraphQL schema federation subgraphs gateway schema stitching and resolvers"),
        ("rest-api-design", "RESTful API design HTTP status codes idempotency pagination and versioning"),
        ("grpc-protobuf", "Protocol Buffers proto3 message schemas service stubs and RPC error codes"),
        ("oauth2-oidc", "OAuth2 authorization code flow OpenID Connect JWT tokens and refresh tokens"),
        ("security-owasp", "OWASP security vulnerabilities SQL injection XSS CSRF and input validation"),
        ("git-branching", "Git rebase interactive merge conflicts cherry-pick and commit history"),
        ("github-actions", "GitHub Actions CI/CD workflows matrix build steps secrets and artifacts"),
        ("gitlab-ci", "GitLab CI yaml pipelines stages runners environments and cache keys"),
        ("db-migrations", "SQL database migration scripts schema versioning rollback and integrity checks"),
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

    let engine = Engine::new_with_store(config, Arc::new(store)).unwrap();

    let negative_queries = [
        "currently getting many false positive skill suggestions, need to rethink how the suggestion is working.",
        "why is git diff showing untracked files?",
        "how does the event loop work conceptually in browser engines?",
        "can we discuss the trade-offs of microservices vs monolith architectures?",
        "what is the best approach to refactor this module without breaking callers?",
        "please review the PR changes and give feedback",
        "could you explain the algorithm time complexity here?",
        "what is the status of the current task?",
        "why did the previous build fail on CI?",
        "rebase from master and check if everything passes",
        "thanks for the help, that resolved it",
        "tell me about the history of programming languages",
        "how do I configure my terminal prompt colors?",
        "let's pause here and summarize our decisions",
        "what does this error message mean in general terms?",
    ];

    for query in &negative_queries {
        let res = engine
            .route_task(
                query,
                Some(ws_dir.path()),
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
            res.recommended_skills.is_empty(),
            "expected 0 skills for negative query {:?}, got: {:?}",
            query,
            res.recommended_skills
        );
    }

    let positive_cases: &[(&str, &str)] = &[
        ("write unit tests with cargo test", "rust-testing"),
        ("idiomatic rust error handling and Result patterns", "rust-patterns"),
        ("asynchronous tokio futures and select channels in rust", "rust-async"),
        ("kubernetes helm chart deployment and ingress configuration", "kubernetes-deploy"),
        ("docker multi-stage build layer caching", "docker-optimization"),
        ("postgres connection pooling with pgbouncer", "postgres-pooling"),
        ("python asyncio coroutines and event loop", "python-asyncio"),
        ("react server components streaming suspense", "react-streaming"),
        ("graphql schema federation and resolvers", "graphql-federation"),
        ("golang goroutines channel synchronization", "go-concurrency"),
        ("typescript strict null checks and type guards", "typescript-types"),
        ("aws terraform infrastructure as code setup", "terraform-aws"),
        ("redis pubsub caching and eviction policy", "redis-caching"),
        ("linux systemd service unit configuration", "linux-systemd"),
        ("jest snapshot testing and mock functions", "jest-testing"),
        ("sql database migration scripts and schema rollback", "db-migrations"),
    ];

    let mut positive_hits = 0;
    for (query, expected_skill) in positive_cases {
        let res = engine
            .route_task(
                query,
                Some(ws_dir.path()),
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

        let hit = res.recommended_skills.iter().any(|s| s.name == *expected_skill);
        assert!(
            hit,
            "expected skill {} for query {:?}, got: {:?}",
            expected_skill,
            query,
            res.recommended_skills
        );
        positive_hits += 1;
    }

    assert_eq!(positive_hits, positive_cases.len());
}
