use anyhow::Result;
use tekmor_core::evidence::EvidenceRecord;

use crate::db::store::BrainStore;

pub fn recent_runs(store: &BrainStore, limit: usize) -> Result<Vec<EvidenceRecord>> {
    Ok(store
        .list_verify_outcomes(limit)?
        .into_iter()
        .map(EvidenceRecord::from)
        .collect())
}

pub fn project_runs(
    store: &BrainStore,
    project_key: &str,
    limit: usize,
) -> Result<Vec<EvidenceRecord>> {
    Ok(store
        .list_verify_outcomes_for_project(project_key, limit)?
        .into_iter()
        .map(EvidenceRecord::from)
        .collect())
}
