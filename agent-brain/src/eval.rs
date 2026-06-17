//! Retrieval eval harness for CI gates (Recall@3).
//!
//! Routing accuracy is the product USP — both memory and skill suites must pass.

use std::sync::Arc;

use anyhow::{bail, Result};

use crate::db::store::{content_hash, BrainStore};
use crate::embed::deterministic_embedding;
use crate::engine::Engine;
use crate::types::{ItemType, RouteLimits};

pub const RECALL_AT_3_THRESHOLD: f64 = 0.85;

#[derive(Debug, Clone, serde::Serialize)]
pub struct EvalReport {
    pub memory: SuiteResult,
    pub skills: SuiteResult,
    /// Combined case count (memory + skills) for backward-compatible tooling.
    pub cases: usize,
    pub passed: usize,
    pub recall_at_3: f64,
    pub threshold: f64,
    pub failures: Vec<EvalFailure>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SuiteResult {
    pub suite: &'static str,
    pub cases: usize,
    pub passed: usize,
    pub recall_at_3: f64,
    pub failures: Vec<EvalFailure>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EvalFailure {
    pub suite: &'static str,
    pub query: String,
    pub expected_topics: Vec<String>,
    pub got_topics: Vec<String>,
}

struct MemoryGoldenCase {
    query: &'static str,
    fact: &'static str,
    topic: &'static str,
}

struct SkillGoldenCase {
    query: &'static str,
    topic: &'static str,
    text: &'static str,
}

const MEMORY_GOLDEN: &[MemoryGoldenCase] = &[
    MemoryGoldenCase {
        query: "configure vitest for react testing",
        fact: "Do not use Jest for this project; prefer Vitest",
        topic: "testing-framework",
    },
    MemoryGoldenCase {
        query: "postgres connection pool settings",
        fact: "Use PgBouncer in transaction mode for serverless Postgres",
        topic: "postgres-pooling",
    },
    MemoryGoldenCase {
        query: "rust error handling patterns",
        fact: "Prefer anyhow::Result in binaries and thiserror in libraries",
        topic: "rust-errors",
    },
    MemoryGoldenCase {
        query: "mcp server stdio transport",
        fact: "agent-brain MCP servers use stdio transport with rmcp",
        topic: "mcp-transport",
    },
    MemoryGoldenCase {
        query: "api versioning strategy for breaking changes",
        fact: "Use URL path versioning /v1 for public APIs; never break without deprecation window",
        topic: "api-versioning",
    },
    MemoryGoldenCase {
        query: "never use jest in this repo tests",
        fact: "Do not use Jest — Vitest only for unit and component tests",
        topic: "no-jest",
    },
    MemoryGoldenCase {
        query: "secrets in environment variables not committed",
        fact: "Never commit API keys; load from env or secret_refs only",
        topic: "secrets-policy",
    },
];

const SKILL_GOLDEN: &[SkillGoldenCase] = &[
    SkillGoldenCase {
        query: "review the changes on the PR before merge",
        topic: "code-review",
        text: "When to activate: reviewing pull request diffs, merge readiness, and review checklists",
    },
    SkillGoldenCase {
        query: "write react component tests with vitest",
        topic: "react-testing",
        text: "When to use: React component testing with Vitest, RTL, MSW, and accessibility assertions",
    },
    SkillGoldenCase {
        query: "optimize postgres queries and indexing",
        topic: "postgres-patterns",
        text: "PostgreSQL query optimization, schema design, indexing, and connection pooling best practices",
    },
    SkillGoldenCase {
        query: "build a new mcp server with stdio transport",
        topic: "mcp-server-patterns",
        text: "Build MCP servers with Node/TypeScript SDK — tools, Zod validation, stdio vs Streamable HTTP",
    },
    SkillGoldenCase {
        query: "deploy release with health checks and rollback",
        topic: "deployment-patterns",
        text: "Deployment workflows, CI/CD pipelines, Docker, health checks, and production rollback strategies",
    },
    SkillGoldenCase {
        query: "debug failing test with unexpected behavior",
        topic: "systematic-debugging",
        text: "When to use: encountering bugs, test failures, or unexpected behavior before proposing fixes",
    },
    SkillGoldenCase {
        query: "fastapi dependency injection and pydantic schemas",
        topic: "fastapi-patterns",
        text: "FastAPI best practices covering Pydantic v2 schemas, dependency injection, async handlers, and testing",
    },
    SkillGoldenCase {
        query: "golang table driven tests and subtests",
        topic: "golang-testing",
        text: "Go testing patterns including table-driven tests, subtests, benchmarks, fuzzing, and test coverage",
    },
    SkillGoldenCase {
        query: "django rest framework api pagination filtering",
        topic: "django-patterns",
        text: "Django architecture patterns, REST API design with DRF, ORM best practices, caching, and middleware",
    },
    SkillGoldenCase {
        query: "nestjs modules guards interceptors production api",
        topic: "nestjs-patterns",
        text: "NestJS architecture patterns for modules, controllers, providers, DTO validation, guards, and interceptors",
    },
    SkillGoldenCase {
        query: "security review authentication authorization checklist",
        topic: "security-review",
        text: "Use when adding authentication, handling user input, secrets, API endpoints, or payment features",
    },
    SkillGoldenCase {
        query: "write a new cursor agent skill SKILL.md",
        topic: "writing-skills",
        text: "Guides users through creating effective Agent Skills for Cursor with SKILL.md structure and best practices",
    },
    SkillGoldenCase {
        query: "grep before cat large log file token efficient",
        topic: "token-efficient-ops",
        text: "Token-efficient shell and file inspection — grep, head, and bounded reads before full file loads",
    },
];

/// Decoy skills that should not win unrelated queries.
const SKILL_DECOYS: &[(&str, &str)] = &[
    (
        "cooking-tips",
        "Weeknight pasta recipes, baking tips, and sauce ideas for home cooks",
    ),
    (
        "brand-voice",
        "Build writing style profiles from blog posts for marketing and outreach copy",
    ),
    (
        "investor-outreach",
        "Draft cold emails and warm intro blurbs for fundraising with angels and VCs",
    ),
];

pub fn run_ci_eval(engine: &Engine) -> Result<EvalReport> {
    seed_eval_fixture(&engine.store)?;
    run_ci_eval_seeded(engine)
}

/// Isolated temp DB with deterministic embeddings — used by CI and published proofs.
pub fn run_ci_eval_isolated() -> Result<EvalReport> {
    let (engine, _dir) = crate::fixture::new_isolated_engine()?;
    run_ci_eval(&engine)
}

pub fn run_ci_eval_seeded(engine: &Engine) -> Result<EvalReport> {
    let memory = run_memory_suite(engine)?;
    let skills = run_skill_suite(engine)?;

    let cases = memory.cases + skills.cases;
    let passed = memory.passed + skills.passed;
    let recall_at_3 = if cases == 0 {
        1.0
    } else {
        passed as f64 / cases as f64
    };
    let mut failures = memory.failures.clone();
    failures.extend(skills.failures.clone());

    Ok(EvalReport {
        memory,
        skills,
        cases,
        passed,
        recall_at_3,
        threshold: RECALL_AT_3_THRESHOLD,
        failures,
    })
}

pub fn assert_ci_gate(report: &EvalReport) -> Result<()> {
    if report.memory.recall_at_3 < RECALL_AT_3_THRESHOLD {
        bail!(
            "memory Recall@3 {:.2} below threshold {:.2} ({} / {} passed)",
            report.memory.recall_at_3,
            RECALL_AT_3_THRESHOLD,
            report.memory.passed,
            report.memory.cases
        );
    }
    if report.skills.recall_at_3 < RECALL_AT_3_THRESHOLD {
        bail!(
            "skills Recall@3 {:.2} below threshold {:.2} ({} / {} passed)",
            report.skills.recall_at_3,
            RECALL_AT_3_THRESHOLD,
            report.skills.passed,
            report.skills.cases
        );
    }
    Ok(())
}

fn run_memory_suite(engine: &Engine) -> Result<SuiteResult> {
    let limits = RouteLimits {
        agents: 0,
        skills: 0,
        rules: 0,
        memory: 5,
    };
    run_suite(engine, "memory", limits, |resp| {
        resp.relevant_memory
            .iter()
            .take(3)
            .map(|m| m.topic.clone())
            .collect()
    })
}

fn run_skill_suite(engine: &Engine) -> Result<SuiteResult> {
    let limits = RouteLimits {
        agents: 0,
        skills: 3,
        rules: 0,
        memory: 0,
    };
    run_suite(engine, "skills", limits, |resp| {
        resp.recommended_skills
            .iter()
            .take(3)
            .map(|s| s.name.clone())
            .collect()
    })
}

fn run_suite<F>(
    engine: &Engine,
    suite: &'static str,
    limits: RouteLimits,
    got_topics: F,
) -> Result<SuiteResult>
where
    F: Fn(&crate::types::RouteTaskResponse) -> Vec<String>,
{
    let golden: Vec<(&str, &str)> = if suite == "memory" {
        MEMORY_GOLDEN
            .iter()
            .map(|c| (c.query, c.topic))
            .collect()
    } else {
        SKILL_GOLDEN
            .iter()
            .map(|c| (c.query, c.topic))
            .collect()
    };

    let mut passed = 0usize;
    let mut failures = Vec::new();

    for (query, expected_topic) in golden {
        let resp = engine.route_task(query, None, &[], 500, limits, Some("implementing"))?;
        let topics = got_topics(&resp);
        if topics.iter().any(|t| t == expected_topic) {
            passed += 1;
        } else {
            failures.push(EvalFailure {
                suite,
                query: query.to_string(),
                expected_topics: vec![expected_topic.to_string()],
                got_topics: topics,
            });
        }
    }

    let cases = if suite == "memory" {
        MEMORY_GOLDEN.len()
    } else {
        SKILL_GOLDEN.len()
    };
    let recall_at_3 = if cases == 0 {
        1.0
    } else {
        passed as f64 / cases as f64
    };

    Ok(SuiteResult {
        suite,
        cases,
        passed,
        recall_at_3,
        failures,
    })
}

fn seed_golden_facts(store: &Arc<BrainStore>) -> Result<()> {
    for case in MEMORY_GOLDEN {
        let emb = deterministic_embedding(case.fact);
        let hash = content_hash(case.fact);
        store.store_fact(
            case.topic,
            case.fact,
            "global",
            None,
            0.95,
            "eval",
            &hash,
            &emb,
            "positive",
        )?;
    }
    store.bump_index_version()?;
    Ok(())
}

fn seed_golden_skills(store: &Arc<BrainStore>) -> Result<()> {
    for case in SKILL_GOLDEN {
        upsert_skill(store, case.topic, case.text)?;
    }
    for (topic, text) in SKILL_DECOYS {
        upsert_skill(store, topic, text)?;
    }
    store.bump_index_version()?;
    Ok(())
}

/// Golden memory + skill decoys for eval and bench proofs.
pub fn seed_eval_fixture(store: &Arc<BrainStore>) -> Result<()> {
    seed_golden_facts(store)?;
    seed_golden_skills(store)?;
    Ok(())
}

/// Filler skills to reach a target index size for latency benchmarks.
pub fn seed_filler_skills(store: &Arc<BrainStore>, count: usize) -> Result<()> {
    for i in 0..count {
        let topic = format!("bench-filler-{i:04}");
        let text = format!(
            "Generic documentation and utility patterns for module {i}; unrelated to pull requests, testing, or deployment"
        );
        upsert_skill(store, &topic, &text)?;
    }
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
    )?;
    Ok(())
}
