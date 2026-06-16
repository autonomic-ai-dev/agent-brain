//! Latency benchmarks on an isolated fixture index (deterministic embedder).
//!
//! Published with `agent-brain proofs --ci`; thresholds are enforced in CI.

use std::sync::Arc;

use anyhow::{bail, Result};

use crate::engine::Engine;
use crate::fixture::{default_route_limits, new_isolated_engine, seed_bench_fixture, BENCH_FIXTURE_SKILLS};

pub const TURN_CACHE_HIT_P95_THRESHOLD_MS: u64 = 30;
pub const WARM_ROUTE_P95_THRESHOLD_MS: u64 = 100;

const WARMUP_ROUTES: usize = 5;
const SAMPLES: usize = 25;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PercentileMs {
    pub samples: usize,
    pub min_ms: u64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LatencyBenchReport {
    pub fixture_skills: usize,
    pub warmup_routes: usize,
    pub turn_cache_hit: PercentileMs,
    pub warm_route: PercentileMs,
    pub turn_cache_hit_p95_threshold_ms: u64,
    pub warm_route_p95_threshold_ms: u64,
    pub passed: bool,
}

pub fn run_ci_bench() -> Result<LatencyBenchReport> {
    let (engine, _dir) = new_isolated_engine()?;
    let skills = seed_bench_fixture(&engine.store, BENCH_FIXTURE_SKILLS)?;
    run_bench_on_engine(&engine, skills)
}

pub fn run_bench_on_engine(engine: &Engine, fixture_skills: usize) -> Result<LatencyBenchReport> {
    let limits = default_route_limits();
    let query = "configure vitest for react testing";

    for i in 0..WARMUP_ROUTES {
        let q = format!("{query} warmup {i}");
        engine.route_task(&q, None, &[], 500, limits, Some("implementing"))?;
    }

    let mut cache_hits = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let resp = engine.route_task(query, None, &[], 500, limits, Some("implementing"))?;
        cache_hits.push(resp.latency_ms);
    }

    let mut warm_routes = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let q = format!("implement rust backend service module {i} with error handling");
        let resp = engine.route_task(&q, None, &[], 500, limits, Some("implementing"))?;
        warm_routes.push(resp.latency_ms);
    }

    let turn_cache_hit = percentiles(&cache_hits);
    let warm_route = percentiles(&warm_routes);
    let passed = turn_cache_hit.p95_ms <= TURN_CACHE_HIT_P95_THRESHOLD_MS
        && warm_route.p95_ms <= WARM_ROUTE_P95_THRESHOLD_MS;

    Ok(LatencyBenchReport {
        fixture_skills,
        warmup_routes: WARMUP_ROUTES,
        turn_cache_hit,
        warm_route,
        turn_cache_hit_p95_threshold_ms: TURN_CACHE_HIT_P95_THRESHOLD_MS,
        warm_route_p95_threshold_ms: WARM_ROUTE_P95_THRESHOLD_MS,
        passed,
    })
}

pub fn assert_bench_gate(report: &LatencyBenchReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    if report.turn_cache_hit.p95_ms > report.turn_cache_hit_p95_threshold_ms {
        bail!(
            "turn cache hit p95 {}ms exceeds threshold {}ms",
            report.turn_cache_hit.p95_ms,
            report.turn_cache_hit_p95_threshold_ms
        );
    }
    bail!(
        "warm route p95 {}ms exceeds threshold {}ms",
        report.warm_route.p95_ms,
        report.warm_route_p95_threshold_ms
    );
}

fn percentiles(samples: &[u64]) -> PercentileMs {
    let mut v: Vec<u64> = samples.to_vec();
    v.sort_unstable();
    let len = v.len();
    let idx = |pct: f64| -> usize {
        if len == 0 {
            return 0;
        }
        (((len as f64) * pct).ceil() as usize)
            .saturating_sub(1)
            .min(len - 1)
    };
    PercentileMs {
        samples: len,
        min_ms: *v.first().unwrap_or(&0),
        p50_ms: v[idx(0.50)],
        p95_ms: v[idx(0.95)],
        max_ms: *v.last().unwrap_or(&0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_orders_samples() {
        let p = percentiles(&[10, 20, 30, 40, 50]);
        assert_eq!(p.p50_ms, 30);
        assert!(p.p95_ms >= 40);
    }
}
