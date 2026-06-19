//! Graphify ingest + route_task `code_context` latency benchmarks.

use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Result};
use serde_json::json;

use crate::bench::{percentiles, PercentileMs};
use crate::fixture::{default_route_limits, new_isolated_engine, seed_bench_fixture, BENCH_FIXTURE_SKILLS};
use crate::graphify::ingest_graph_at_path;

pub const GRAPHIFY_INGEST_1K_THRESHOLD_MS: u64 = 2_000;
pub const GRAPHIFY_ROUTE_P95_THRESHOLD_MS: u64 = 65;
pub const GRAPHIFY_SIZES: &[usize] = &[100, 1_000, 5_000];

const ROUTE_SAMPLES: usize = 25;
const ROUTE_WARMUP: usize = 3;

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphifyBenchTier {
    pub node_count: usize,
    pub edge_count: usize,
    pub ingest_ms: u64,
    pub route_with_code_context: PercentileMs,
    pub code_context_hit_rate: f64,
    pub ingest_threshold_ms: u64,
    pub route_p95_threshold_ms: u64,
    pub passed: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphifyBenchReport {
    pub fixture_skills: usize,
    pub tiers: Vec<GraphifyBenchTier>,
    pub passed: bool,
}

pub fn run_graphify_bench(sizes: &[usize]) -> Result<GraphifyBenchReport> {
    let mut tiers = Vec::new();
    for &size in sizes {
        tiers.push(run_graphify_tier(size)?);
    }
    let passed = tiers.iter().all(|t| t.passed);
    Ok(GraphifyBenchReport {
        fixture_skills: BENCH_FIXTURE_SKILLS,
        tiers,
        passed,
    })
}

pub fn run_ci_graphify_bench() -> Result<GraphifyBenchReport> {
    run_graphify_bench(&[1_000])
}

pub fn run_graphify_tier(node_count: usize) -> Result<GraphifyBenchTier> {
    let (engine, dir) = new_isolated_engine()?;
    seed_bench_fixture(&engine.store, BENCH_FIXTURE_SKILLS)?;
    let repo = dir.path().join("bench-repo");
    fs::create_dir_all(&repo)?;
    let edge_count = write_synthetic_graph(&repo, node_count)?;

    let ingest_started = Instant::now();
    ingest_graph_at_path(&engine.store, &repo)?;
    let ingest_ms = ingest_started.elapsed().as_millis() as u64;

    let limits = default_route_limits();
    let query_base = "navigate authentication module database calls";
    for i in 0..ROUTE_WARMUP {
        let q = format!("{query_base} warmup {i}");
        engine.route_task(&q, Some(&repo), &[], 500, limits, Some("implementing"), None)?;
    }

    let mut samples = Vec::with_capacity(ROUTE_SAMPLES);
    let mut hits = 0usize;
    for i in 0..ROUTE_SAMPLES {
        let q = format!("{query_base} trace flow {i}");
        let resp = engine.route_task(&q, Some(&repo), &[], 500, limits, Some("implementing"), None)?;
        if resp.code_context.is_some() {
            hits += 1;
        }
        samples.push(resp.latency_ms);
    }

    let route_with_code_context = percentiles(&samples);
    let code_context_hit_rate = hits as f64 / ROUTE_SAMPLES as f64;
    let ingest_threshold = match node_count {
        n if n >= 5_000 => 5_000,
        n if n >= 1_000 => GRAPHIFY_INGEST_1K_THRESHOLD_MS,
        _ => 500,
    };
    let ingest_ok = ingest_ms <= ingest_threshold;
    let route_ok = route_with_code_context.p95_ms <= GRAPHIFY_ROUTE_P95_THRESHOLD_MS;
    let passed = ingest_ok && route_ok && code_context_hit_rate >= 0.95;

    Ok(GraphifyBenchTier {
        node_count,
        edge_count,
        ingest_ms,
        route_with_code_context,
        code_context_hit_rate,
        ingest_threshold_ms: ingest_threshold,
        route_p95_threshold_ms: GRAPHIFY_ROUTE_P95_THRESHOLD_MS,
        passed,
    })
}

pub fn write_synthetic_graph(repo_root: &Path, node_count: usize) -> Result<usize> {
    let out = repo_root.join("graphify-out");
    fs::create_dir_all(&out)?;
    let nodes: Vec<serde_json::Value> = (0..node_count)
        .map(|i| {
            json!({
                "id": format!("Node{i}"),
                "label": format!("Module{i}"),
                "community": i % 12,
                "source_file": format!("src/module_{i}.rs"),
            })
        })
        .collect();
    let mut links = Vec::new();
    for i in 0..node_count.saturating_sub(1) {
        links.push(json!({
            "source": format!("Node{i}"),
            "target": format!("Node{}", i + 1),
            "relation": "calls",
        }));
    }
    for i in (0..node_count).step_by(7) {
        if i + 7 < node_count {
            links.push(json!({
                "source": format!("Node{i}"),
                "target": format!("Node{}", i + 7),
                "relation": "imports",
            }));
        }
    }
    let edge_count = links.len();
    fs::write(
        out.join("graph.json"),
        serde_json::to_string(&json!({ "nodes": nodes, "links": links }))?,
    )?;
    let gods: Vec<String> = (0..5.min(node_count)).map(|i| format!("Module{i}")).collect();
    fs::write(
        out.join(".graphify_analysis.json"),
        serde_json::to_string(&json!({ "gods": gods }))?,
    )?;
    Ok(edge_count)
}

pub fn assert_graphify_bench_gate(report: &GraphifyBenchReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    for tier in &report.tiers {
        if !tier.passed {
            bail!(
                "graphify bench {} nodes failed: ingest {}ms (threshold {}ms), \
                 route p95 {}ms (threshold {}ms), code_context hit {:.0}%",
                tier.node_count,
                tier.ingest_ms,
                tier.ingest_threshold_ms,
                tier.route_with_code_context.p95_ms,
                tier.route_p95_threshold_ms,
                tier.code_context_hit_rate * 100.0
            );
        }
    }
    bail!("graphify bench failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphify_1k_ci_gate() {
        let report = run_ci_graphify_bench().unwrap();
        assert!(report.tiers[0].code_context_hit_rate >= 0.95);
        assert_graphify_bench_gate(&report).unwrap();
    }
}
