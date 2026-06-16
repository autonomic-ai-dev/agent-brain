//! Combined eval + latency proofs for CI and published benchmark artifacts.

use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::bench::{assert_bench_gate, run_ci_bench, LatencyBenchReport};
use crate::eval::{assert_ci_gate, run_ci_eval_isolated, EvalReport};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProofReport {
    pub generated_at: String,
    pub environment: &'static str,
    pub embedder: &'static str,
    pub fixture_skills: usize,
    pub eval: EvalReport,
    pub latency: LatencyBenchReport,
    pub passed: bool,
}

pub fn run_ci_proofs() -> Result<ProofReport> {
    let eval = run_ci_eval_isolated()?;
    assert_ci_gate(&eval)?;

    let latency = run_ci_bench()?;
    assert_bench_gate(&latency)?;

    Ok(ProofReport {
        generated_at: Utc::now().to_rfc3339(),
        environment: "isolated-fixture",
        embedder: "deterministic",
        fixture_skills: latency.fixture_skills,
        passed: true,
        eval,
        latency,
    })
}

pub fn write_proof_report(path: &Path, report: &ProofReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn assert_ci_proofs(report: &ProofReport) -> Result<()> {
    assert_ci_gate(&report.eval)?;
    assert_bench_gate(&report.latency)?;
    if !report.passed {
        bail!("proof report marked failed");
    }
    Ok(())
}
