//! Combined eval + latency + supervisor proofs for CI and published benchmark artifacts.

use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::beam_eval::{assert_beam_gate, run_beam_eval_isolated, BeamEvalReport};
use crate::bench::{assert_bench_gate, run_ci_bench, LatencyBenchReport};
use crate::eval::{assert_ci_gate, run_ci_eval_isolated, EvalReport};
use crate::graphify_bench::{
    assert_graphify_bench_gate, run_ci_graphify_bench, GraphifyBenchReport,
};
use crate::scale_bench::{assert_scale_bench_gate, run_ci_scale_bench, ScaleBenchReport};
use crate::supervisor_bench::{
    assert_supervisor_bench_gate, run_supervisor_bench, SupervisorBenchReport,
};
use crate::token_tools::{run_token_tools_bench, TokenToolsBenchReport};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProofReport {
    pub generated_at: String,
    pub environment: &'static str,
    pub embedder: &'static str,
    pub fixture_skills: usize,
    pub eval: EvalReport,
    pub beam: BeamEvalReport,
    pub latency: LatencyBenchReport,
    pub supervisor: SupervisorBenchReport,
    pub token_tools: TokenToolsBenchReport,
    pub scale: ScaleBenchReport,
    pub graphify: GraphifyBenchReport,
    pub passed: bool,
}

pub fn run_ci_proofs() -> Result<ProofReport> {
    let eval = run_ci_eval_isolated()?;
    assert_ci_gate(&eval)?;

    let beam = run_beam_eval_isolated()?;
    assert_beam_gate(&beam)?;

    let latency = run_ci_bench()?;
    assert_bench_gate(&latency)?;

    let supervisor = run_supervisor_bench()?;
    assert_supervisor_bench_gate(&supervisor)?;

    let token_tools = run_token_tools_bench()?;
    if !token_tools.passed {
        anyhow::bail!(
            "token tools bench failed: min {:.0}% savings required",
            token_tools.savings_min_pct
        );
    }

    let scale = run_ci_scale_bench()?;
    assert_scale_bench_gate(&scale)?;

    let graphify = run_ci_graphify_bench()?;
    assert_graphify_bench_gate(&graphify)?;

    let report = ProofReport {
        generated_at: Utc::now().to_rfc3339(),
        environment: "isolated-fixture",
        embedder: "deterministic",
        fixture_skills: latency.fixture_skills,
        passed: true,
        eval,
        beam,
        latency,
        supervisor,
        token_tools,
        scale,
        graphify,
    };
    crate::proofs_reb_baseline::assert_reb_baseline(&report, None)?;
    Ok(report)
}

pub fn write_proof_report(path: &Path, report: &ProofReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    if let Some(parent) = path.parent() {
        let supervisor_path = parent.join("supervisor-latest.json");
        write_supervisor_report(&supervisor_path, &report.supervisor)?;
    }
    Ok(())
}

pub fn write_supervisor_report(path: &Path, report: &SupervisorBenchReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn assert_ci_proofs(report: &ProofReport) -> Result<()> {
    assert_ci_gate(&report.eval)?;
    assert_beam_gate(&report.beam)?;
    assert_bench_gate(&report.latency)?;
    assert_supervisor_bench_gate(&report.supervisor)?;
    if !report.token_tools.passed {
        bail!("token tools proof gate failed");
    }
    assert_scale_bench_gate(&report.scale)?;
    assert_graphify_bench_gate(&report.graphify)?;
    crate::proofs_reb_baseline::assert_reb_baseline(report, None)?;
    if !report.passed {
        bail!("proof report marked failed");
    }
    Ok(())
}
