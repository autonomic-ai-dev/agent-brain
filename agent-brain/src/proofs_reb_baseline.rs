//! Reb baseline gate: `proofs --ci` must not regress vs committed golden metrics.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::proofs::ProofReport;

#[derive(Debug, Deserialize)]
struct RebBaseline {
    eval: RebEvalBaseline,
}

#[derive(Debug, Deserialize)]
struct RebEvalBaseline {
    recall_at_3: f64,
    cases: usize,
}

const DEFAULT_BASELINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/benchmarks/latest.json");

pub fn assert_reb_baseline(report: &ProofReport, baseline_path: Option<&Path>) -> Result<()> {
    let path = baseline_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Path::new(DEFAULT_BASELINE).to_path_buf());
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("read reb baseline {}", path.display()))?;
    let baseline: RebBaseline = serde_json::from_str(&body)
        .with_context(|| format!("parse reb baseline {}", path.display()))?;

    const MAX_REGRESSION: f64 = 0.02;
    let floor = (baseline.eval.recall_at_3 - MAX_REGRESSION).max(0.0);
    if report.eval.recall_at_3 + f64::EPSILON < floor {
        bail!(
            "reb baseline regression: recall@3 {:.3} < floor {:.3} (baseline {:.3})",
            report.eval.recall_at_3,
            floor,
            baseline.eval.recall_at_3
        );
    }
    if report.eval.cases < baseline.eval.cases {
        bail!(
            "reb baseline regression: eval cases {} < baseline {}",
            report.eval.cases,
            baseline.eval.cases
        );
    }
    Ok(())
}
