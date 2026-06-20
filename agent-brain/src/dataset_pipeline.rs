use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::dataset;
use crate::db::store::BrainStore;
use crate::spine_ingest::{self, IngestReport};

#[derive(Debug, Clone, Default)]
pub struct PipelineGates {
    pub verify_ui_url: Option<String>,
    pub verify_memory_script: Option<PathBuf>,
    pub ui_threshold: f64,
    pub memory_threshold_kb: u64,
}

#[derive(Debug, Serialize)]
pub struct GateReport {
    pub ui: Option<serde_json::Value>,
    pub memory: Option<serde_json::Value>,
    pub passed: bool,
}

#[derive(Debug, Serialize)]
pub struct PipelineReport {
    pub spine_ingest: IngestReport,
    pub trajectory_entries: usize,
    pub merged_entries: u64,
    pub merged_path: PathBuf,
    pub gates: GateReport,
}

pub fn run_gates(gates: &PipelineGates) -> Result<GateReport> {
    let mut ui_result = None;
    let mut memory_result = None;
    let mut passed = true;

    if let Some(url) = &gates.verify_ui_url {
        let eyes = find_binary("agent-eyes").context("agent-eyes not found for UI gate")?;
        let output = Command::new(&eyes)
            .args([
                "verify",
                url,
                "--threshold",
                &gates.ui_threshold.to_string(),
            ])
            .output()
            .context("run agent-eyes verify")?;
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).unwrap_or(serde_json::json!({
                "passed": output.status.success(),
                "stdout": String::from_utf8_lossy(&output.stdout),
            }));
        passed &= output.status.success()
            && json
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(output.status.success());
        ui_result = Some(json);
    }

    if let Some(script) = &gates.verify_memory_script {
        let immune =
            find_binary("agent-immune").context("agent-immune not found for memory gate")?;
        let output = Command::new(&immune)
            .args([
                "verify-memory",
                &script.display().to_string(),
                "--threshold-kb",
                &gates.memory_threshold_kb.to_string(),
            ])
            .output()
            .context("run agent-immune verify-memory")?;
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).unwrap_or(serde_json::json!({
                "passed": output.status.success(),
                "stdout": String::from_utf8_lossy(&output.stdout),
            }));
        passed &= output.status.success()
            && json
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(output.status.success());
        memory_result = Some(json);
    }

    Ok(GateReport {
        ui: ui_result,
        memory: memory_result,
        passed,
    })
}

pub fn run_pipeline(
    store: &BrainStore,
    out: &Path,
    only_successful: bool,
    min_confidence: f64,
    gates: &PipelineGates,
) -> Result<PipelineReport> {
    let gate_report = run_gates(gates)?;
    if !gate_report.passed {
        anyhow::bail!("dataset pipeline gates failed (UI/memory verification)");
    }

    let spine_report = spine_ingest::ingest_executions(None, out, only_successful)?;
    let traj_entries = dataset::export_dataset(store, min_confidence, only_successful)?;
    let traj_path = out.with_extension("trajectories.jsonl");
    {
        use std::io::Write;
        let file = std::fs::File::create(&traj_path)?;
        let mut writer = std::io::BufWriter::new(file);
        for entry in &traj_entries {
            let line = serde_json::to_string(entry)?;
            writeln!(writer, "{line}")?;
        }
        writer.flush()?;
    }
    let merged_path = out.with_extension("merged.jsonl");
    let merged = spine_ingest::merge_jsonl_files(&[out.to_path_buf(), traj_path], &merged_path)?;

    Ok(PipelineReport {
        spine_ingest: spine_report,
        trajectory_entries: traj_entries.len(),
        merged_entries: merged,
        merged_path,
        gates: gate_report,
    })
}

fn find_binary(name: &str) -> Result<PathBuf> {
    if let Ok(path) = std::env::var(format!(
        "AUTONOMIC_{}_BINARY",
        name.to_uppercase().replace('-', "_")
    )) {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
    }
    for path in [
        format!("/usr/local/bin/{name}"),
        format!("/opt/homebrew/bin/{name}"),
    ] {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Ok(p);
        }
    }
    if let Ok(output) = Command::new("which").arg(name).output() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    anyhow::bail!("{name} binary not found on PATH")
}
