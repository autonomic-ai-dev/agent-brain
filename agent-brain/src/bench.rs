//! Latency benchmarks on an isolated fixture index (deterministic embedder).
//!
//! Published with `agent-brain proofs --ci`; thresholds are enforced in CI.
//! ONNX warm-route bench uses `fixture-2k.db` for index crowding (informational / nightly).

use std::path::Path;

use anyhow::{bail, Result};

use crate::engine::Engine;
use crate::fixture::{
    default_fixture_2k_path, open_fixture_engine_with_onnx,
    seed_bench_fixture, BENCH_FIXTURE_SKILLS,
};
use crate::fixture::{default_route_limits, new_isolated_engine};

pub const TURN_CACHE_HIT_P95_THRESHOLD_MS: u64 = 30;
pub const WARM_ROUTE_P95_THRESHOLD_MS: u64 = 100;
/// Design-target warm-route p95 with real ONNX query embedder (not PR-blocking on all runners).
pub const ONNX_WARM_ROUTE_P95_TARGET_MS: u64 = 50;

const WARMUP_ROUTES: usize = 5;
const ONNX_WARMUP_ROUTES: usize = 10;
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct OnnxBenchReport {
    pub environment: String,
    pub embedder: String,
    pub fixture_db: String,
    pub index_size: usize,
    pub snapshot_skills: usize,
    pub filler_skills: usize,
    pub warmup_routes: usize,
    pub warm_route: PercentileMs,
    pub turn_cache_hit: PercentileMs,
    pub warm_route_p95_target_ms: u64,
    pub passed_target: bool,
    pub note: String,
}

pub fn run_ci_bench() -> Result<LatencyBenchReport> {
    let (engine, _dir) = new_isolated_engine()?;
    let skills = seed_bench_fixture(&engine.store, BENCH_FIXTURE_SKILLS)?;
    run_bench_on_engine(&engine, skills, WARMUP_ROUTES)
}

pub fn run_onnx_fixture_bench(fixture_path: &Path) -> Result<OnnxBenchReport> {
    let meta = crate::fixture::read_fixture_meta(fixture_path)?;
    let breakdown = {
        let dir = tempfile::tempdir()?;
        let config = crate::config::Config::isolated(dir.path().to_path_buf());
        config.ensure_dirs()?;
        std::fs::copy(fixture_path, &config.db_path)?;
        let store = crate::db::store::BrainStore::open(&config.db_path)?;
        crate::fixture::fixture_db_breakdown(&store)?
    };
    let (engine, _dir) = open_fixture_engine_with_onnx(fixture_path, true)?;
    let embedder = engine.embedder.model_id.to_string();
    let bench = run_bench_on_engine(&engine, meta.index_size, ONNX_WARMUP_ROUTES)?;
    let passed_target = bench.warm_route.p95_ms <= ONNX_WARM_ROUTE_P95_TARGET_MS;
    Ok(OnnxBenchReport {
        environment: "fixture-2k-onnx".into(),
        embedder,
        fixture_db: fixture_path.display().to_string(),
        index_size: meta.index_size,
        snapshot_skills: breakdown.snapshot_skills,
        filler_skills: breakdown.filler_skills,
        warmup_routes: ONNX_WARMUP_ROUTES,
        warm_route: bench.warm_route,
        turn_cache_hit: bench.turn_cache_hit,
        warm_route_p95_target_ms: ONNX_WARM_ROUTE_P95_TARGET_MS,
        passed_target,
        note: "Query embed uses ONNX; indexed skill vectors are deterministic (latency proxy, not semantic parity).".into(),
    })
}

pub fn run_onnx_fixture_bench_default() -> Result<OnnxBenchReport> {
    let path = default_fixture_2k_path();
    if !path.exists() {
        bail!(
            "missing {} — run: agent-brain fixture build",
            path.display()
        );
    }
    run_onnx_fixture_bench(&path)
}

pub fn assert_onnx_bench_target(report: &OnnxBenchReport) -> Result<()> {
    if report.passed_target {
        return Ok(());
    }
    bail!(
        "ONNX warm-route p95 {}ms exceeds design target {}ms on {}",
        report.warm_route.p95_ms,
        report.warm_route_p95_target_ms,
        report.fixture_db
    );
}

pub fn run_bench_on_engine(
    engine: &Engine,
    fixture_skills: usize,
    warmup_routes: usize,
) -> Result<LatencyBenchReport> {
    let limits = default_route_limits();
    let query = "configure vitest for react testing";

    for i in 0..warmup_routes {
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
        warmup_routes,
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

pub fn percentiles(samples: &[u64]) -> PercentileMs {
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
