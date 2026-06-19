//! BEAM-scale eval harness — memory, routing, temporal, must_apply, and observation suites.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::db::store::{content_hash, BrainStore, FactTemporal};
use crate::embed::deterministic_embedding;
use crate::engine::Engine;
use crate::eval::{run_ci_eval_seeded, seed_eval_fixture, EvalReport, SuiteResult};
use crate::observation::{run_observations, ObservationConfig};
use crate::types::{ItemType, RouteLimits, RouteTaskResponse};

pub const BEAM_THRESHOLD: f64 = 0.85;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BeamEvalReport {
    pub recall: EvalReport,
    pub temporal: SuiteResult,
    pub must_apply: SuiteResult,
    pub observation: SuiteResult,
    pub jsonl: SuiteResult,
    pub transcript: SuiteResult,
    pub task_scoped: SuiteResult,
    pub escalation_signal: SuiteResult,
    pub task_scoped_verification: SuiteResult,
    pub overall_cases: usize,
    pub overall_passed: usize,
    pub overall_score: f64,
    pub threshold: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct JsonlCase {
    query: String,
    #[serde(default)]
    _cwd_hint: Option<String>,
    expect_types: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TaskScopedCase {
    query: String,
    #[serde(default)]
    task_kind: Option<String>,
    expect_task_kind: String,
    #[serde(default)]
    expect_escalate: Option<bool>,
    #[serde(default = "default_true")]
    expect_bundle: bool,
}

fn default_true() -> bool {
    true
}

pub fn run_beam_eval_isolated() -> Result<BeamEvalReport> {
    let (engine, _dir) = crate::fixture::new_isolated_engine()?;
    run_beam_eval(&engine)
}

pub fn run_beam_eval(engine: &Engine) -> Result<BeamEvalReport> {
    seed_eval_fixture(&engine.store)?;
    seed_beam_extensions(&engine.store)?;
    engine.store.bump_index_version()?;

    let recall = run_ci_eval_seeded(engine)?;
    let temporal = run_temporal_suite(engine)?;
    let must_apply = run_must_apply_suite(engine)?;
    let observation = run_observation_suite(engine)?;
    let jsonl = run_jsonl_suite(engine)?;
    let transcript = run_transcript_suite(engine)?;
    let task_scoped = run_task_scoped_suite(engine)?;
    let escalation_signal = run_escalation_signal_suite(engine)?;
    let task_scoped_verification = run_task_scoped_verification_suite(engine)?;

    let suites = [
        &temporal,
        &must_apply,
        &observation,
        &jsonl,
        &transcript,
        &task_scoped,
        &escalation_signal,
        &task_scoped_verification,
    ];
    let extension_cases: usize = suites.iter().map(|s| s.cases).sum();
    let extension_passed: usize = suites.iter().map(|s| s.passed).sum();
    let overall_cases = recall.cases + extension_cases;
    let overall_passed = recall.passed + extension_passed;
    let overall_score = if overall_cases == 0 {
        1.0
    } else {
        overall_passed as f64 / overall_cases as f64
    };

    Ok(BeamEvalReport {
        recall,
        temporal,
        must_apply,
        observation,
        jsonl,
        transcript,
        task_scoped,
        escalation_signal,
        task_scoped_verification,
        overall_cases,
        overall_passed,
        overall_score,
        threshold: BEAM_THRESHOLD,
    })
}

pub fn assert_beam_gate(report: &BeamEvalReport) -> Result<()> {
    crate::eval::assert_ci_gate(&report.recall)?;
    for suite in [
        &report.temporal,
        &report.must_apply,
        &report.observation,
        &report.jsonl,
        &report.transcript,
        &report.task_scoped,
        &report.escalation_signal,
        &report.task_scoped_verification,
    ] {
        if suite.cases > 0 && suite.recall_at_3 < BEAM_THRESHOLD {
            bail!(
                "BEAM suite '{}' score {:.2} below threshold {:.2}",
                suite.suite,
                suite.recall_at_3,
                BEAM_THRESHOLD
            );
        }
    }
    if report.overall_score < BEAM_THRESHOLD {
        bail!(
            "BEAM overall {:.2} below threshold {:.2} ({} / {} passed)",
            report.overall_score,
            BEAM_THRESHOLD,
            report.overall_passed,
            report.overall_cases
        );
    }
    Ok(())
}

fn run_temporal_suite(engine: &Engine) -> Result<SuiteResult> {
    let limits = RouteLimits {
        agents: 0,
        skills: 0,
        rules: 0,
        memory: 5,
    };
    let mut passed = 0usize;
    let mut failures = Vec::new();

    let resp = engine.route_task(
        "duckdb analytics warehouse for this project",
        None,
        &[],
        500,
        limits,
        Some("implementing"),
        None,
    )?;
    let topics: Vec<String> = resp
        .relevant_memory
        .iter()
        .take(3)
        .map(|m| m.topic.clone())
        .collect();
    if topics.iter().any(|t| t == "beam-active-analytics") {
        passed += 1;
    } else {
        failures.push(crate::eval::EvalFailure {
            suite: "temporal",
            query: "duckdb analytics warehouse".into(),
            expected_topics: vec!["beam-active-analytics".into()],
            got_topics: topics.clone(),
        });
    }

    let resp = engine.route_task(
        "sqlite analytics warehouse legacy setup",
        None,
        &[],
        500,
        limits,
        Some("implementing"),
        None,
    )?;
    let topics: Vec<String> = resp
        .relevant_memory
        .iter()
        .take(3)
        .map(|m| m.topic.clone())
        .collect();
    if !topics.iter().any(|t| t == "beam-expired-analytics") {
        passed += 1;
    } else {
        failures.push(crate::eval::EvalFailure {
            suite: "temporal",
            query: "sqlite analytics legacy".into(),
            expected_topics: vec!["!beam-expired-analytics".into()],
            got_topics: topics,
        });
    }

    Ok(suite_result("temporal", 2, passed, failures))
}

fn run_must_apply_suite(engine: &Engine) -> Result<SuiteResult> {
    let limits = RouteLimits {
        agents: 0,
        skills: 0,
        rules: 0,
        memory: 5,
    };
    let cases = [
        ("configure vitest for react testing", "testing-framework"),
        ("debug async race condition in handler", "debugging"),
    ];
    let mut passed = 0usize;
    let mut failures = Vec::new();
    for (query, expected_topic) in cases {
        let resp = engine.route_task(query, None, &[], 500, limits, Some("debugging"), None)?;
        let hit = resp
            .must_apply
            .iter()
            .any(|m| m.topic == expected_topic)
            || resp
                .relevant_memory
                .iter()
                .any(|m| m.topic == expected_topic);
        if hit {
            passed += 1;
        } else {
            failures.push(crate::eval::EvalFailure {
                suite: "must_apply",
                query: query.to_string(),
                expected_topics: vec![expected_topic.to_string()],
                got_topics: resp.must_apply.iter().map(|m| m.topic.clone()).collect(),
            });
        }
    }
    Ok(suite_result("must_apply", cases.len(), passed, failures))
}

fn run_observation_suite(engine: &Engine) -> Result<SuiteResult> {
    let embedder = crate::embed::Embedder::deterministic();
    let report = run_observations(
        &engine.store,
        &embedder,
        &ObservationConfig::default(),
        false,
    )?;
    engine.store.bump_index_version()?;
    let limits = RouteLimits {
        agents: 0,
        skills: 0,
        rules: 0,
        memory: 5,
    };
    let passed = if report.synthesized > 0 {
        let resp = engine.route_task(
            "add vitest tests for react components",
            None,
            &[],
            500,
            limits,
            Some("implementing"),
            None,
        )?;
        let hit = resp.must_apply.iter().any(|m| m.topic.starts_with("obs/"))
            || resp
                .relevant_memory
                .iter()
                .any(|m| m.topic.starts_with("obs/"));
        usize::from(hit)
    } else {
        0
    };
    let failures = if passed == 1 {
        Vec::new()
    } else {
        vec![crate::eval::EvalFailure {
            suite: "observation",
            query: "observation synthesis".into(),
            expected_topics: vec!["obs/testing".into()],
            got_topics: vec![],
        }]
    };
    Ok(suite_result("observation", 1, passed, failures))
}

fn run_jsonl_suite(engine: &Engine) -> Result<SuiteResult> {
    run_jsonl_file_suite(engine, jsonl_fixture_path(), "jsonl")
}

fn run_transcript_suite(engine: &Engine) -> Result<SuiteResult> {
    run_jsonl_file_suite(engine, transcript_fixture_path(), "transcript")
}

fn run_task_scoped_suite(engine: &Engine) -> Result<SuiteResult> {
    let path = task_scoped_fixture_path();
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cases: Vec<TaskScopedCase> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l))
        .collect::<Result<Vec<_>, _>>()?;

    let limits = RouteLimits::default();
    let mut passed = 0usize;
    let mut failures = Vec::new();
    for case in &cases {
        let resp = engine.route_task(
            &case.query,
            None,
            &[],
            500,
            limits,
            None,
            case.task_kind.as_deref(),
        )?;
        let kind_ok = resp.task_kind.as_deref() == Some(case.expect_task_kind.as_str());
        let bundle_ok = !case.expect_bundle || resp.context_bundle.is_some();
        let escalate_ok = case
            .expect_escalate
            .map(|want| resp.escalate_recommended == want)
            .unwrap_or(true);
        let confidence_ok = resp.route_confidence > 0.0
            || case.expect_escalate == Some(true)
            || resp.escalate_recommended;
        if kind_ok && bundle_ok && escalate_ok && confidence_ok {
            passed += 1;
        } else {
            failures.push(crate::eval::EvalFailure {
                suite: "task_scoped",
                query: case.query.clone(),
                expected_topics: vec![
                    format!("task_kind={}", case.expect_task_kind),
                    format!("escalate={:?}", case.expect_escalate),
                ],
                got_topics: vec![
                    format!("task_kind={:?}", resp.task_kind),
                    format!("escalate={}", resp.escalate_recommended),
                    format!("confidence={:.2}", resp.route_confidence),
                ],
            });
        }
    }
    Ok(suite_result("task_scoped", cases.len(), passed, failures))
}

fn run_escalation_signal_suite(engine: &Engine) -> Result<SuiteResult> {
    seed_escalation_fixture(&engine.store)?;
    engine.store.bump_index_version()?;
    let mut passed = 0usize;
    let mut failures = Vec::new();

    let (empty_engine, _dir) = crate::fixture::new_isolated_engine()?;
    let empty_resp = empty_engine.route_task(
        "zzzz verification noop without indexed context",
        None,
        &[],
        500,
        RouteLimits::default(),
        None,
        Some("verification"),
    )?;
    if empty_resp.escalate_recommended {
        passed += 1;
    } else {
        failures.push(crate::eval::EvalFailure {
            suite: "escalation_signal",
            query: "empty index verification".into(),
            expected_topics: vec!["escalate=true".into()],
            got_topics: vec![format!(
                "escalate={} confidence={:.2}",
                empty_resp.escalate_recommended, empty_resp.route_confidence
            )],
        });
    }

    let resp = engine.route_task(
        "verify BEAM proofs and eval --beam in CI before merge",
        None,
        &[],
        500,
        RouteLimits::default(),
        None,
        Some("verification"),
    )?;
    if !resp.escalate_recommended {
        passed += 1;
    } else {
        failures.push(crate::eval::EvalFailure {
            suite: "escalation_signal",
            query: "verify BEAM proofs and eval --beam in CI before merge".into(),
            expected_topics: vec!["escalate=false".into()],
            got_topics: vec![format!(
                "escalate={} confidence={:.2} rules={} memory={}",
                resp.escalate_recommended,
                resp.route_confidence,
                resp.applicable_rules.len(),
                resp.relevant_memory.len()
            )],
        });
    }

    Ok(suite_result("escalation_signal", 2, passed, failures))
}

fn run_task_scoped_verification_suite(engine: &Engine) -> Result<SuiteResult> {
    seed_escalation_fixture(&engine.store)?;
    engine.store.bump_index_version()?;
    let limits = RouteLimits {
        agents: 2,
        skills: 3,
        rules: 2,
        memory: 5,
    };
    let mut passed = 0usize;
    let mut failures = Vec::new();
    let resp = engine.route_task(
        "verify BEAM proofs and eval --beam in CI before merge",
        None,
        &[],
        500,
        limits,
        None,
        Some("verification"),
    )?;
    let mut ok = resp.task_kind.as_deref() == Some("verification")
        && resp.recommended_agents.len() <= 1
        && resp.recommended_skills.len() <= 2
        && resp.relevant_memory.len() <= 3;
    if ok {
        passed += 1;
    } else {
        failures.push(crate::eval::EvalFailure {
            suite: "task_scoped_verification",
            query: "verify BEAM proofs pass in CI".into(),
            expected_topics: vec![
                "agents<=1".into(),
                "skills<=2".into(),
                "memory<=3".into(),
            ],
            got_topics: vec![
                format!("agents={}", resp.recommended_agents.len()),
                format!("skills={}", resp.recommended_skills.len()),
                format!("memory={}", resp.relevant_memory.len()),
            ],
        });
    }

    let (empty_engine, _dir) = crate::fixture::new_isolated_engine()?;
    let empty = empty_engine.route_task(
        "zzzz verification noop without indexed context",
        None,
        &[],
        500,
        RouteLimits::default(),
        None,
        Some("verification"),
    )?;
    if empty.escalate_recommended && empty.context_bundle.is_some() {
        passed += 1;
    } else {
        failures.push(crate::eval::EvalFailure {
            suite: "task_scoped_verification",
            query: "empty verification context".into(),
            expected_topics: vec!["escalate=true".into(), "bundle=some".into()],
            got_topics: vec![
                format!("escalate={}", empty.escalate_recommended),
                format!("bundle={}", empty.context_bundle.is_some()),
            ],
        });
    }

    Ok(suite_result(
        "task_scoped_verification",
        2,
        passed,
        failures,
    ))
}

fn run_jsonl_file_suite(engine: &Engine, path: std::path::PathBuf, suite: &'static str) -> Result<SuiteResult> {
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cases: Vec<JsonlCase> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l))
        .collect::<Result<Vec<_>, _>>()?;

    let limits = RouteLimits {
        agents: 2,
        skills: 3,
        rules: 2,
        memory: 3,
    };
    let mut passed = 0usize;
    let mut failures = Vec::new();
    for case in &cases {
        let resp = engine.route_task(
            &case.query,
            None,
            &[],
            500,
            limits,
            Some("implementing"),
            None,
        )?;
        let present = response_types(&resp);
        let ok = case
            .expect_types
            .iter()
            .all(|t| present.contains(&t.to_ascii_lowercase()));
        if ok {
            passed += 1;
        } else {
            failures.push(crate::eval::EvalFailure {
                suite,
                query: case.query.clone(),
                expected_topics: case.expect_types.clone(),
                got_topics: present.into_iter().collect(),
            });
        }
    }
    Ok(suite_result(suite, cases.len(), passed, failures))
}

fn suite_result(
    suite: &'static str,
    cases: usize,
    passed: usize,
    failures: Vec<crate::eval::EvalFailure>,
) -> SuiteResult {
    SuiteResult {
        suite,
        cases,
        passed,
        recall_at_3: if cases == 0 {
            1.0
        } else {
            passed as f64 / cases as f64
        },
        failures,
    }
}

fn response_types(resp: &RouteTaskResponse) -> HashSet<String> {
    let mut out = HashSet::new();
    if !resp.recommended_skills.is_empty() {
        out.insert("skill".into());
    }
    if !resp.applicable_rules.is_empty() {
        out.insert("rule".into());
    }
    if !resp.recommended_agents.is_empty() {
        out.insert("agent".into());
    }
    if !resp.relevant_memory.is_empty() {
        out.insert("memory".into());
    }
    out
}

fn jsonl_fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/queries.jsonl")
}

fn transcript_fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/transcript-queries.jsonl")
}

fn task_scoped_fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/task-scoped.jsonl")
}

fn seed_beam_extensions(store: &Arc<BrainStore>) -> Result<()> {
    seed_temporal_cases(store)?;
    seed_must_apply_extension(store)?;
    seed_observation_inputs(store)?;
    seed_jsonl_fixture(store)?;
    Ok(())
}

fn seed_temporal_cases(store: &Arc<BrainStore>) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let active = "Use DuckDB for analytics warehouse workloads in this project";
    let emb = deterministic_embedding(active);
    store.store_fact_full(
        "beam-active-analytics",
        active,
        "global",
        None,
        0.95,
        "eval",
        &content_hash(active),
        &emb,
        "positive",
        None,
        Some(&FactTemporal {
            valid_from: Some(now - 10_000),
            invalid_at: None,
        }),
    )?;

    let expired = "Use SQLite for analytics warehouse legacy setup (expired)";
    let emb = deterministic_embedding(expired);
    store.store_fact_full(
        "beam-expired-analytics",
        expired,
        "global",
        None,
        0.9,
        "eval",
        &content_hash(expired),
        &emb,
        "positive",
        None,
        Some(&FactTemporal {
            valid_from: Some(now - 20_000),
            invalid_at: Some(now - 1),
        }),
    )?;
    Ok(())
}

fn seed_must_apply_extension(store: &Arc<BrainStore>) -> Result<()> {
    let text = "Check race conditions in async handlers before shipping";
    let emb = deterministic_embedding(text);
    store.store_fact_full(
        "debugging",
        text,
        "global",
        None,
        0.9,
        "agent",
        &content_hash(text),
        &emb,
        "positive",
        Some(r#"["phase:debugging"]"#),
        None,
    )?;
    Ok(())
}

fn seed_observation_inputs(store: &Arc<BrainStore>) -> Result<()> {
    let emb = vec![0.04; 384];
    for i in 0..3 {
        let fact = format!("Prefer Vitest over Jest for component tests variant {i}");
        store.store_fact_full(
            "testing",
            &fact,
            "global",
            None,
            0.9,
            "agent",
            &content_hash(&fact),
            &emb,
            "positive",
            Some(r#"["phase:implementing"]"#),
            None,
        )?;
    }
    Ok(())
}

fn seed_jsonl_fixture(store: &Arc<BrainStore>) -> Result<()> {
    upsert_rule(
        store,
        "verify-ci-beam",
        "Run BEAM proofs and eval --beam in CI before merge for routing changes",
    )?;
    upsert_skill(
        store,
        "rust-testing",
        "Rust unit and integration testing with cargo test, #[test], and mockall",
    )?;
    upsert_rule(
        store,
        "rust-test-policy",
        "Run cargo test in agent-brain for Rust changes and failing test fixes",
    )?;
    upsert_skill(
        store,
        "security-review",
        "Security review checklist for auth, input validation, secrets, and API endpoints",
    )?;
    upsert_agent(
        store,
        "code-reviewer",
        "Reviews pull requests for correctness, security, and style before merge",
    )?;
    upsert_agent(
        store,
        "mcp-implementer",
        "Implements MCP servers, route_task tools, and agent-brain handlers for IDE agents",
    )?;
    upsert_rule(
        store,
        "mcp-route-task",
        "route_task MCP tool must be called before other agent-brain tools each turn",
    )?;
    upsert_skill(
        store,
        "mcp-server-patterns",
        "Build MCP servers with tools, schemas, and stdio transport for IDE agents",
    )?;

    let mem = "ADD-only memory deduplicates by content_hash without destructive supersede";
    let emb = deterministic_embedding(mem);
    store.store_fact(
        "memory-dedup",
        mem,
        "global",
        None,
        0.95,
        "eval",
        &content_hash(mem),
        &emb,
        "positive",
    )?;

    let err = "Prefer anyhow::Result in binaries and thiserror in libraries for errors";
    let emb = deterministic_embedding(err);
    store.store_fact(
        "error-handling",
        err,
        "global",
        None,
        0.9,
        "eval",
        &content_hash(err),
        &emb,
        "positive",
    )?;

    let store_mem = "Store durable project conventions about error handling via store_memory at task end";
    let emb = deterministic_embedding(store_mem);
    store.store_fact(
        "store-memory",
        store_mem,
        "global",
        None,
        0.92,
        "eval",
        &content_hash(store_mem),
        &emb,
        "positive",
    )?;
    Ok(())
}

fn seed_escalation_fixture(store: &Arc<BrainStore>) -> Result<()> {
    upsert_rule(
        store,
        "verify-ci-beam",
        "Run BEAM proofs and eval --beam in CI before merge for routing changes",
    )?;
    upsert_skill(
        store,
        "beam-ci",
        "BEAM eval harness runs proofs CI regression gates for agent-brain routing",
    )?;
    upsert_agent(
        store,
        "ci-verifier",
        "Verifies BEAM proofs and eval --beam pass before merge",
    )?;
    let mem = "Always run agent-brain eval --beam in CI before merging routing changes";
    let emb = deterministic_embedding(mem);
    store.store_fact(
        "ci-beam",
        mem,
        "global",
        None,
        0.95,
        "eval",
        &content_hash(mem),
        &emb,
        "positive",
    )?;
    Ok(())
}

fn upsert_skill(store: &Arc<BrainStore>, topic: &str, text: &str) -> Result<()> {
    let emb = deterministic_embedding(&format!("{topic} {text}"));
    let hash = content_hash(text);
    store.upsert_indexed_item(
        ItemType::Skill,
        topic,
        text,
        &format!("/skills/{topic}/SKILL.md"),
        "global",
        None,
        &hash,
        Some(&emb),
    )
}

fn upsert_agent(store: &Arc<BrainStore>, topic: &str, text: &str) -> Result<()> {
    let emb = deterministic_embedding(&format!("{topic} {text}"));
    let hash = content_hash(text);
    store.upsert_indexed_item(
        ItemType::Agent,
        topic,
        text,
        &format!("/agents/{topic}.md"),
        "global",
        None,
        &hash,
        Some(&emb),
    )
}

fn upsert_rule(store: &Arc<BrainStore>, topic: &str, text: &str) -> Result<()> {
    let emb = deterministic_embedding(&format!("{topic} {text}"));
    let hash = content_hash(text);
    store.upsert_indexed_item(
        ItemType::Rule,
        topic,
        text,
        &format!("/rules/{topic}.mdc"),
        "global",
        None,
        &hash,
        Some(&emb),
    )
}
