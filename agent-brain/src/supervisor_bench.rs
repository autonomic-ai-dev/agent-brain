//! Execution supervisor efficiency benchmarks — skill recall, must_apply, token savings, latency.
//!
//! Run: `agent-brain bench --supervisor [--assert]`

use std::sync::Arc;

use anyhow::{bail, Result};
use tempfile::TempDir;

use crate::bench::percentiles;
use crate::config::Config;
use crate::db::store::{content_hash, BrainStore};
use crate::embed::deterministic_embedding;
use crate::engine::Engine;
use crate::fixture::{seed_bench_fixture, BENCH_FIXTURE_SKILLS};
use crate::packages::install_bundled;
use crate::route_briefing::token_savings;
use crate::token_tools::{run_token_tools_bench, TokenToolsBenchReport};
use crate::types::RouteLimits;

pub const SUPERVISOR_SKILL_HIT_THRESHOLD: f64 = 1.0;
pub const MUST_APPLY_HIT_THRESHOLD: f64 = 1.0;
pub const SUPERVISOR_WARM_P95_MS: u64 = 100;
pub const AVG_SAVED_PCT_MIN: usize = 90;

const WARMUP_ROUTES: usize = 3;
const LATENCY_SAMPLES: usize = 15;

struct Scenario {
    query: &'static str,
    expect_skill: Option<&'static str>,
    expect_must_apply: Option<&'static str>,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        query: "grep before cat a large log file token efficient",
        expect_skill: Some("token-efficient-ops"),
        expect_must_apply: None,
    },
    Scenario {
        query: "read frontend build artifacts from dist folder deployment",
        expect_skill: None,
        expect_must_apply: Some("no-read-dist"),
    },
    Scenario {
        query: "use rg head tail instead of cat huge files save tokens",
        expect_skill: Some("token-efficient-ops"),
        expect_must_apply: None,
    },
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct SupervisorScenarioResult {
    pub query: String,
    pub expect_skill: Option<String>,
    pub expect_must_apply_topic: Option<String>,
    pub skill_hit: bool,
    pub must_apply_hit: bool,
    pub saved_pct: usize,
    pub latency_ms: u64,
    pub bm25_fast_path_eligible: bool,
    pub tokens_used: usize,
    pub must_apply_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SupervisorBenchReport {
    pub fixture_skills: usize,
    pub scenarios: Vec<SupervisorScenarioResult>,
    pub skill_hit_rate: f64,
    pub must_apply_rate: f64,
    pub avg_saved_pct: f64,
    pub bm25_fast_path_rate: f64,
    pub warm_route_p95_ms: u64,
    pub skill_hit_threshold: f64,
    pub must_apply_threshold: f64,
    pub warm_route_p95_threshold_ms: u64,
    pub avg_saved_pct_min: usize,
    pub token_tools: TokenToolsBenchReport,
    pub passed: bool,
}

pub fn run_supervisor_bench() -> Result<SupervisorBenchReport> {
    let (engine, _dir) = setup_supervisor_engine()?;
    run_supervisor_bench_on_engine(&engine)
}

fn setup_supervisor_engine() -> Result<(Arc<Engine>, TempDir)> {
    let dir = tempfile::tempdir()?;
    let mut config = Config::isolated(dir.path().to_path_buf());
    config.bm25_fast_path_enabled = true;
    config.ensure_dirs()?;
    install_bundled(&config, "supervisor")?;

    let store = Arc::new(BrainStore::open(&config.db_path)?);
    let fact = "Never read dist/ or build output directories with cat or read_file";
    let emb = deterministic_embedding(fact);
    let hash = content_hash(fact);
    store.store_fact(
        "no-read-dist",
        fact,
        "global",
        None,
        0.95,
        "eval",
        &hash,
        &emb,
        "negative",
    )?;
    let skills = seed_bench_fixture(&store, BENCH_FIXTURE_SKILLS)?;
    store.bump_index_version()?;

    let engine = Arc::new(Engine::new_with_store(config, store)?);
    let indexed = engine.bootstrap(None)?;
    if indexed < 3 {
        bail!(
            "expected supervisor pack indexed, got {indexed} (skills seeded: {skills})"
        );
    }
    Ok((engine, dir))
}

pub fn run_supervisor_bench_on_engine(engine: &Engine) -> Result<SupervisorBenchReport> {
    let limits = RouteLimits {
        agents: 0,
        skills: 3,
        rules: 2,
        memory: 3,
    };

    for i in 0..WARMUP_ROUTES {
        let q = format!("supervisor warmup route {i}");
        engine.route_task(&q, None, &[], 500, limits, Some("implementing"))?;
    }

    let mut scenarios = Vec::with_capacity(SCENARIOS.len());
    let mut skill_checks = 0usize;
    let mut skill_hits = 0usize;
    let mut must_apply_checks = 0usize;
    let mut must_apply_hits = 0usize;
    let mut saved_pcts = Vec::new();
    let mut fast_path_eligible = 0usize;

    for scenario in SCENARIOS {
        let bm25 = engine.store.bm25_prefilter(scenario.query)?;
        let eligible = bm25.fast_path_eligible(scenario.query);
        if eligible {
            fast_path_eligible += 1;
        }

        let resp = engine.route_task(
            scenario.query,
            None,
            &[],
            500,
            limits,
            Some("implementing"),
        )?;

        let saved_pct = token_savings(&resp).map(|s| s.saved_pct).unwrap_or(0);
        saved_pcts.push(saved_pct);

        let skill_hit = scenario.expect_skill.is_none_or(|expected| {
            resp.recommended_skills
                .iter()
                .any(|s| s.name == expected)
        });
        if scenario.expect_skill.is_some() {
            skill_checks += 1;
            if skill_hit {
                skill_hits += 1;
            }
        }

        let must_apply_hit = scenario.expect_must_apply.is_none_or(|expected| {
            resp.must_apply.iter().any(|m| m.topic == expected)
        });
        if scenario.expect_must_apply.is_some() {
            must_apply_checks += 1;
            if must_apply_hit {
                must_apply_hits += 1;
            }
        }

        scenarios.push(SupervisorScenarioResult {
            query: scenario.query.to_string(),
            expect_skill: scenario.expect_skill.map(str::to_string),
            expect_must_apply_topic: scenario.expect_must_apply.map(str::to_string),
            skill_hit,
            must_apply_hit,
            saved_pct,
            latency_ms: resp.latency_ms,
            bm25_fast_path_eligible: eligible,
            tokens_used: resp.tokens_used,
            must_apply_count: resp.must_apply.len(),
        });
    }

    let mut warm_latencies = Vec::with_capacity(LATENCY_SAMPLES);
    for i in 0..LATENCY_SAMPLES {
        let q = format!("grep rg token efficient large log file scenario {i}");
        let resp = engine.route_task(&q, None, &[], 500, limits, Some("implementing"))?;
        warm_latencies.push(resp.latency_ms);
    }
    let warm_route_p95_ms = percentiles(&warm_latencies).p95_ms;

    let skill_hit_rate = if skill_checks == 0 {
        1.0
    } else {
        skill_hits as f64 / skill_checks as f64
    };
    let must_apply_rate = if must_apply_checks == 0 {
        1.0
    } else {
        must_apply_hits as f64 / must_apply_checks as f64
    };
    let avg_saved_pct = if saved_pcts.is_empty() {
        0.0
    } else {
        saved_pcts.iter().sum::<usize>() as f64 / saved_pcts.len() as f64
    };
    let bm25_fast_path_rate = fast_path_eligible as f64 / SCENARIOS.len() as f64;

    let token_tools = run_token_tools_bench()?;

    let passed = skill_hit_rate >= SUPERVISOR_SKILL_HIT_THRESHOLD
        && must_apply_rate >= MUST_APPLY_HIT_THRESHOLD
        && avg_saved_pct >= AVG_SAVED_PCT_MIN as f64
        && warm_route_p95_ms <= SUPERVISOR_WARM_P95_MS
        && token_tools.passed;

    Ok(SupervisorBenchReport {
        fixture_skills: BENCH_FIXTURE_SKILLS,
        scenarios,
        skill_hit_rate,
        must_apply_rate,
        avg_saved_pct,
        bm25_fast_path_rate,
        warm_route_p95_ms,
        skill_hit_threshold: SUPERVISOR_SKILL_HIT_THRESHOLD,
        must_apply_threshold: MUST_APPLY_HIT_THRESHOLD,
        warm_route_p95_threshold_ms: SUPERVISOR_WARM_P95_MS,
        avg_saved_pct_min: AVG_SAVED_PCT_MIN,
        token_tools,
        passed,
    })
}

pub fn assert_supervisor_bench_gate(report: &SupervisorBenchReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    if report.skill_hit_rate < report.skill_hit_threshold {
        bail!(
            "supervisor skill hit rate {:.0}% below threshold {:.0}%",
            report.skill_hit_rate * 100.0,
            report.skill_hit_threshold * 100.0
        );
    }
    if report.must_apply_rate < report.must_apply_threshold {
        bail!(
            "must_apply hit rate {:.0}% below threshold {:.0}%",
            report.must_apply_rate * 100.0,
            report.must_apply_threshold * 100.0
        );
    }
    if report.avg_saved_pct < report.avg_saved_pct_min as f64 {
        bail!(
            "avg saved_pct {:.0}% below minimum {}%",
            report.avg_saved_pct,
            report.avg_saved_pct_min
        );
    }
    if !report.token_tools.passed {
        bail!(
            "token tools avg savings {:.0}% below minimum {:.0}%",
            report.token_tools.avg_savings_pct,
            report.token_tools.savings_min_pct
        );
    }
    bail!(
        "supervisor warm route p95 {}ms exceeds threshold {}ms",
        report.warm_route_p95_ms,
        report.warm_route_p95_threshold_ms
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_bench_passes_on_fixture() {
        let report = run_supervisor_bench().expect("supervisor bench");
        assert!(
            report.passed,
            "supervisor bench failed: {:?}",
            report.scenarios
        );
        assert_supervisor_bench_gate(&report).expect("gate");
    }
}
