//! Archive stale memory facts using context_weights feedback signals.

use anyhow::Result;

use crate::db::store::{BrainStore, GcCandidate};

#[derive(Debug, Clone, serde::Serialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub candidates: usize,
    pub archived: usize,
    pub skipped_protected: usize,
    pub ids: Vec<String>,
}

pub fn run_memory_gc(store: &BrainStore, dry_run: bool, force: bool) -> Result<GcReport> {
    run_memory_gc_with_thresholds(store, dry_run, force, 90, 180)
}

pub fn run_memory_gc_with_thresholds(
    store: &BrainStore,
    dry_run: bool,
    force: bool,
    stale_days: u32,
    very_stale_days: u32,
) -> Result<GcReport> {
    let now = chrono::Utc::now().timestamp_millis();
    let stale_ms = i64::from(stale_days) * 24 * 3600 * 1000;
    let very_stale_ms = i64::from(very_stale_days) * 24 * 3600 * 1000;

    let candidates = store.list_gc_candidates(now, stale_ms, very_stale_ms)?;
    let mut archived = 0usize;
    let mut skipped_protected = 0usize;
    let mut ids = Vec::new();

    for candidate in candidates {
        if is_protected(&candidate) && !force {
            skipped_protected += 1;
            continue;
        }
        ids.push(candidate.id.clone());
        if !dry_run {
            store.archive_fact(&candidate, "stale_low_signal")?;
            archived += 1;
        }
    }

    Ok(GcReport {
        dry_run,
        candidates: ids.len() + skipped_protected,
        archived: if dry_run { 0 } else { archived },
        skipped_protected,
        ids,
    })
}

fn is_protected(candidate: &GcCandidate) -> bool {
    if candidate.polarity.as_deref() == Some("negative") {
        return true;
    }
    if candidate.apply_when.is_some() {
        return true;
    }
    if candidate.source.as_deref() == Some("user") && candidate.confidence >= 0.95 {
        return true;
    }
    false
}
