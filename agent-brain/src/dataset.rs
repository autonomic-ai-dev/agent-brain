use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::store::BrainStore;

#[derive(Debug, Serialize, Deserialize)]
pub struct DatasetEntry {
    pub instruction: String,
    pub response: String,
    pub workflow_name: String,
    pub node_kind: String,
    pub model: String,
    pub outcome: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize)]
pub struct DatasetStats {
    pub total: u64,
    pub successful: u64,
    pub per_workflow: HashMap<String, u64>,
    pub per_model: HashMap<String, u64>,
    pub date_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

pub fn export_dataset(
    store: &BrainStore,
    _min_confidence: f64,
    _only_successful: bool,
) -> Result<Vec<DatasetEntry>> {
    let trajectories = store.query_trajectories()?;
    let mut entries = Vec::new();

    for traj in trajectories {
        let instruction = format!(
            "Workflow: {} | Node: {} | Task: {}",
            traj.workflow_id,
            traj.node_id,
            traj.task_kind.as_deref().unwrap_or_default(),
        );
        let response = traj.notes.unwrap_or_default();

        let timestamp = DateTime::from_timestamp_millis(traj.recorded_at).unwrap_or_default();

        entries.push(DatasetEntry {
            instruction,
            response,
            workflow_name: traj.workflow_id,
            node_kind: String::new(),
            model: String::new(),
            outcome: traj.outcome,
            timestamp,
        });
    }

    Ok(entries)
}

pub fn compute_stats(entries: &[DatasetEntry]) -> DatasetStats {
    let mut stats = DatasetStats::default();
    stats.total = entries.len() as u64;
    stats.successful = entries.iter().filter(|e| e.outcome == "success").count() as u64;

    for entry in entries {
        *stats.per_workflow.entry(entry.workflow_name.clone()).or_insert(0) += 1;
        *stats.per_model.entry(entry.model.clone()).or_insert(0) += 1;
    }

    if let (Some(first), Some(last)) = (entries.first(), entries.last()) {
        stats.date_range = Some((first.timestamp, last.timestamp));
    }

    stats
}
