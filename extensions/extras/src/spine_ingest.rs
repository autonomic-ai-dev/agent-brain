use agent_brain::global_workspace::{ensure_dirs, executions_dir, memory_dir};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::dataset::DatasetEntry;

#[derive(Debug, Deserialize)]
struct ExecutionGraph {
    execution_id: String,
    workflow_name: String,
    #[allow(dead_code)]
    snapshot_count: usize,
    snapshots: Vec<SnapshotRecord>,
}

#[derive(Debug, Deserialize)]
struct SnapshotRecord {
    sequence: u64,
    transition: Option<TransitionRecord>,
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct TransitionRecord {
    from: String,
    to: String,
}

#[derive(Debug, Default, Serialize)]
pub struct IngestReport {
    pub files_scanned: u64,
    pub files_ingested: u64,
    pub entries_written: u64,
    pub skipped_empty: u64,
    pub output_path: PathBuf,
}

pub fn default_dataset_path() -> PathBuf {
    memory_dir().join("datasets").join("spine.jsonl")
}

pub fn ingest_executions(
    executions_root: Option<&Path>,
    out: &Path,
    only_successful: bool,
) -> Result<IngestReport> {
    ensure_dirs().context("ensure autonomic dirs")?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let root = executions_root
        .map(Path::to_path_buf)
        .unwrap_or_else(executions_dir);

    let mut report = IngestReport {
        output_path: out.to_path_buf(),
        ..Default::default()
    };

    let mut writer = BufWriter::new(File::create(out)?);

    for entry in std::fs::read_dir(&root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".dag.json"))
        {
            continue;
        }

        report.files_scanned += 1;
        let graph: ExecutionGraph = match std::fs::read_to_string(&path)
            .context("read execution graph")
            .and_then(|s| serde_json::from_str(&s).context("parse execution graph"))
        {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skip invalid execution graph");
                continue;
            }
        };

        if graph.snapshots.is_empty() {
            report.skipped_empty += 1;
            continue;
        }

        let mut wrote_any = false;
        for snap in &graph.snapshots {
            let outcome = snap
                .payload
                .get("outcome")
                .or_else(|| snap.payload.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            if only_successful && outcome != "success" {
                continue;
            }

            let (from, to) = snap
                .transition
                .as_ref()
                .map(|t| (t.from.as_str(), t.to.as_str()))
                .unwrap_or(("start", "start"));

            let instruction = format!(
                "Workflow: {} | Execution: {} | Transition: {} -> {} | Seq: {}",
                graph.workflow_name, graph.execution_id, from, to, snap.sequence
            );
            let response = serde_json::to_string(&snap.payload).unwrap_or_default();

            let entry = DatasetEntry {
                instruction,
                response,
                workflow_name: graph.workflow_name.clone(),
                node_kind: to.to_string(),
                model: snap
                    .payload
                    .get("model_used")
                    .or_else(|| snap.payload.get("model"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                outcome: outcome.to_string(),
                timestamp: Utc::now(),
            };

            let line = serde_json::to_string(&entry)?;
            writeln!(writer, "{line}")?;
            report.entries_written += 1;
            wrote_any = true;
        }

        if wrote_any {
            report.files_ingested += 1;
        }
    }

    writer.flush()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn ingests_execution_graph_json() {
        let dir = TempDir::new().unwrap();
        let exec_dir = dir.path().join("executions");
        std::fs::create_dir_all(&exec_dir).unwrap();
        let graph = serde_json::json!({
            "execution_id": "exec-1",
            "workflow_name": "demo",
            "snapshot_count": 1,
            "snapshots": [{
                "sequence": 1,
                "transition": { "from": "plan", "to": "execute" },
                "payload": { "outcome": "success", "model_used": "gpt-4" }
            }]
        });
        std::fs::write(
            exec_dir.join("exec-1.json"),
            serde_json::to_string_pretty(&graph).unwrap(),
        )
        .unwrap();

        let out = dir.path().join("out.jsonl");
        let report = ingest_executions(Some(&exec_dir), &out, true).unwrap();
        assert_eq!(report.files_ingested, 1);
        assert_eq!(report.entries_written, 1);
        assert!(out.exists());
    }
}
