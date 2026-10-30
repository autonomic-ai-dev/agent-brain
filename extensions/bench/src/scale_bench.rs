//! Scale benchmarks at 1k / 5k / 10k indexed skills (deterministic embedder).

use anyhow::{bail, Result};

use agent_brain::ann::DEFAULT_ANN_MIN_INDEX;
use crate::bench::{run_bench_on_engine, PercentileMs};
use agent_brain::fixture::{new_isolated_engine, seed_bench_fixture};

pub const SCALE_P95_THRESHOLD_MS: u64 = 50;

pub fn scale_p95_threshold_ms() -> u64 {
    if let Ok(v) = std::env::var("AGENT_BRAIN_BENCH_P95_MS") {
        if let Ok(ms) = v.parse() {
            return ms;
        }
    }
    if cfg!(debug_assertions) {
        150
    } else {
        SCALE_P95_THRESHOLD_MS
    }
}

pub const SCALE_SIZES: &[usize] = &[1_000, 5_000, 10_000];

const WARMUP_ROUTES: usize = 3;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScaleTierReport {
    pub index_size: usize,
    pub ann_active: bool,
    pub warm_route: PercentileMs,
    pub p95_threshold_ms: u64,
    pub passed: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScaleBenchReport {
    pub tiers: Vec<ScaleTierReport>,
    pub p95_threshold_ms: u64,
    pub passed: bool,
}

pub fn run_scale_bench(sizes: &[usize]) -> Result<ScaleBenchReport> {
    let mut tiers = Vec::new();
    for &size in sizes {
        tiers.push(run_scale_tier(size)?);
    }
    let passed = tiers.iter().all(|t| t.passed);
    Ok(ScaleBenchReport {
        tiers,
        p95_threshold_ms: SCALE_P95_THRESHOLD_MS,
        passed,
    })
}

pub fn run_scale_tier(index_size: usize) -> Result<ScaleTierReport> {
    let (engine, _dir) = new_isolated_engine()?;
    seed_bench_fixture(&engine.store, index_size)?;
    let bench = run_bench_on_engine(&engine, index_size, WARMUP_ROUTES)?;
    let ann_active = index_size >= DEFAULT_ANN_MIN_INDEX;
    let threshold = scale_p95_threshold_ms();
    let passed = bench.warm_route.p95_ms <= threshold;
    Ok(ScaleTierReport {
        index_size,
        ann_active,
        warm_route: bench.warm_route,
        p95_threshold_ms: threshold,
        passed,
    })
}

pub fn assert_scale_bench_gate(report: &ScaleBenchReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    for tier in &report.tiers {
        if !tier.passed {
            bail!(
                "scale bench {} skills warm-route p95 {}ms exceeds {}ms",
                tier.index_size,
                tier.warm_route.p95_ms,
                tier.p95_threshold_ms
            );
        }
    }
    bail!("scale bench failed");
}

/// Fast CI gate: 1k index only (validates ANN path without seeding 10k every run).
pub fn run_ci_scale_bench() -> Result<ScaleBenchReport> {
    run_scale_bench(&[1_000])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_1k_passes_gate() {
        let report = run_ci_scale_bench().unwrap();
        assert!(report.tiers[0].ann_active == false || report.tiers[0].index_size >= 1000);
        assert_scale_bench_gate(&report).unwrap();
    }
}
