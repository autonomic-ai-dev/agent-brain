//! End-to-end MCP tool latency report (route, context, token tools, graphify).

use std::time::Instant;

use anyhow::{bail, Result};

use crate::bench::{percentiles, run_ci_bench, LatencyBenchReport, PercentileMs};
use agent_brain::fixture::new_isolated_engine;
use crate::graphify_bench::{run_ci_graphify_bench, GraphifyBenchReport};
use agent_brain::token_tools::{file_summary, grep_search, read_file_head, DEFAULT_MAX_BYTES};
use agent_brain::types::ItemType;

const SAMPLES: usize = 25;

#[derive(Debug, Clone, serde::Serialize)]
pub struct McpToolLatency {
    pub tool: &'static str,
    pub samples: usize,
    pub min_ms: u64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub max_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct McpBenchReport {
    pub route_task: LatencyBenchReport,
    pub get_context: PercentileMs,
    pub tools: Vec<McpToolLatency>,
    pub graphify: GraphifyBenchReport,
    pub route_p95_threshold_ms: u64,
    pub get_context_p95_threshold_ms: u64,
    pub passed: bool,
}

pub const MCP_GET_CONTEXT_P95_THRESHOLD_MS: u64 = 80;

pub fn run_mcp_bench() -> Result<McpBenchReport> {
    let route_task = run_ci_bench()?;
    let get_context = bench_get_context()?;
    let tools = bench_token_tools()?;
    let graphify = run_ci_graphify_bench()?;

    let passed = route_task.passed
        && get_context.p95_ms <= MCP_GET_CONTEXT_P95_THRESHOLD_MS
        && graphify.passed;

    Ok(McpBenchReport {
        route_task,
        get_context,
        tools,
        graphify,
        route_p95_threshold_ms: crate::bench::WARM_ROUTE_P95_THRESHOLD_MS,
        get_context_p95_threshold_ms: MCP_GET_CONTEXT_P95_THRESHOLD_MS,
        passed,
    })
}

pub fn assert_mcp_bench_gate(report: &McpBenchReport) -> Result<()> {
    crate::bench::assert_bench_gate(&report.route_task)?;
    if report.get_context.p95_ms > report.get_context_p95_threshold_ms {
        bail!(
            "get_context p95 {}ms exceeds threshold {}ms",
            report.get_context.p95_ms,
            report.get_context_p95_threshold_ms
        );
    }
    crate::graphify_bench::assert_graphify_bench_gate(&report.graphify)?;
    if !report.passed {
        bail!("mcp bench marked failed");
    }
    Ok(())
}

fn bench_get_context() -> Result<PercentileMs> {
    let (engine, _dir) = new_isolated_engine()?;
    let types = [ItemType::Skill, ItemType::Rule, ItemType::Memory];
    let query = "configure rust backend testing patterns";
    for i in 0..3 {
        let q = format!("{query} warmup {i}");
        engine.get_context(&q, None, 300, &types)?;
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let started = Instant::now();
        let q = format!("{query} sample {i}");
        engine.get_context(&q, None, 300, &types)?;
        samples.push(started.elapsed().as_millis() as u64);
    }
    Ok(percentiles(&samples))
}

fn bench_token_tools() -> Result<Vec<McpToolLatency>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("src").join("sample.log");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let body: String = (1..=1500)
        .map(|i| format!("line {i} benchmark filler\n"))
        .collect();
    std::fs::write(&path, &body)?;

    Ok(vec![
        bench_tool("file_summary", SAMPLES, || {
            let _ = file_summary(&path, false, 500)?;
            Ok(())
        })?,
        bench_tool("read_file_head", SAMPLES, || {
            let _ = read_file_head(&path, 50, DEFAULT_MAX_BYTES, false, 500)?;
            Ok(())
        })?,
        bench_tool("grep_search", SAMPLES, || {
            let _ = grep_search("benchmark filler", &path, 5, false, false, 500)?;
            Ok(())
        })?,
    ])
}

fn bench_tool(
    name: &'static str,
    samples: usize,
    mut f: impl FnMut() -> Result<()>,
) -> Result<McpToolLatency> {
    let mut ms = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        f()?;
        ms.push(started.elapsed().as_millis() as u64);
    }
    let p = percentiles(&ms);
    Ok(McpToolLatency {
        tool: name,
        samples: p.samples,
        min_ms: p.min_ms,
        p50_ms: p.p50_ms,
        p95_ms: p.p95_ms,
        max_ms: p.max_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_bench_runs() {
        let report = run_mcp_bench().unwrap();
        assert!(report.route_task.fixture_skills > 0);
        assert!(!report.tools.is_empty());
    }
}
