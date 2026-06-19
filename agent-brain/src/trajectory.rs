//! Workflow trajectory logging for orchestrator learning loops (v0.24).

use anyhow::{bail, Result};
use serde::Serialize;

use crate::db::store::{BrainStore, TrajectoryRecord};

const ALLOWED_OUTCOMES: &[&str] = &["success", "failure", "escalated", "skipped"];

#[derive(Debug, Clone, Serialize)]
pub struct StoreTrajectoryReport {
    pub id: String,
    pub workflow_id: String,
    pub node_id: String,
    pub outcome: String,
    pub route_log_linked: bool,
}

pub fn normalize_outcome(outcome: &str) -> Result<&'static str> {
    let lower = outcome.trim().to_lowercase();
    ALLOWED_OUTCOMES
        .iter()
        .find(|o| **o == lower)
        .copied()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "outcome must be one of: {}",
                ALLOWED_OUTCOMES.join(", ")
            )
        })
}

pub fn store_trajectory(
    store: &BrainStore,
    workflow_id: &str,
    node_id: &str,
    outcome: &str,
    route_log_id: Option<&str>,
    task_kind: Option<&str>,
    notes: Option<&str>,
) -> Result<StoreTrajectoryReport> {
    let workflow_id = workflow_id.trim();
    let node_id = node_id.trim();
    if workflow_id.is_empty() || node_id.is_empty() {
        bail!("workflow_id and node_id are required");
    }
    if let Some(notes) = notes {
        if notes.split_whitespace().count() > 50 {
            bail!("notes exceeds 50 words");
        }
    }
    let outcome = normalize_outcome(outcome)?;
    let route_log_id = route_log_id.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let record: TrajectoryRecord = store.insert_workflow_trajectory(
        workflow_id,
        node_id,
        outcome,
        route_log_id,
        task_kind.map(str::trim).filter(|s| !s.is_empty()),
        notes.map(str::trim).filter(|s| !s.is_empty()),
    )?;
    Ok(StoreTrajectoryReport {
        id: record.id,
        workflow_id: record.workflow_id,
        node_id: record.node_id,
        outcome: record.outcome,
        route_log_linked: record.route_log_id.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::BrainStore;
    use tempfile::TempDir;

    #[test]
    fn rejects_unknown_outcome() {
        assert!(normalize_outcome("maybe").is_err());
    }

    #[test]
    fn stores_trajectory_with_optional_route_log() {
        let dir = TempDir::new().unwrap();
        let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
        store
            .insert_retrieval_log(
                "route-1",
                "abc",
                "implementing",
                "[]",
                100,
                false,
                false,
                5,
                Some(10),
                Some(50),
                Some(0),
            )
            .unwrap();
        let report = store_trajectory(
            &store,
            "wf-1",
            "implement",
            "success",
            Some("route-1"),
            Some("implementing"),
            Some("tests passed"),
        )
        .unwrap();
        assert_eq!(report.route_log_linked, true);
    }
}
