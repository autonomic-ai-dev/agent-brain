//! Archive stale memory facts using context_weights feedback signals.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::store::{BrainStore, GcCandidate};

#[derive(Debug, Clone, serde::Serialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub candidates: usize,
    pub archived: usize,
    pub skipped_protected: usize,
    pub ids: Vec<String>,
    pub reason_buckets: Vec<GcReasonBucket>,
    pub top_topics: Vec<GcTopicCount>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GcReasonBucket {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GcTopicCount {
    pub topic: String,
    pub count: usize,
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
    let mut bucket_counts: HashMap<String, usize> = HashMap::new();
    let mut topic_counts: HashMap<String, usize> = HashMap::new();

    for candidate in candidates {
        *topic_counts.entry(candidate.topic.clone()).or_default() += 1;

        if let Some(protection) = protection_reason(&candidate) {
            if !force {
                skipped_protected += 1;
                *bucket_counts
                    .entry(format!("protected:{protection}"))
                    .or_default() += 1;
                continue;
            }
            *bucket_counts
                .entry(format!("forced:{protection}"))
                .or_default() += 1;
        }

        let archive_reason = archive_reason_for(&candidate);
        *bucket_counts
            .entry(format!("archive:{archive_reason}"))
            .or_default() += 1;
        ids.push(candidate.id.clone());
        if !dry_run {
            store.archive_fact(&candidate, archive_reason)?;
            archived += 1;
        }
    }

    let confidence_decay = store.with_conn(|conn| {
        apply_temporal_confidence_decay(conn, chrono::Utc::now().timestamp_millis())
    })?;
    let _ = confidence_decay;

    Ok(GcReport {
        dry_run,
        candidates: ids.len() + skipped_protected,
        archived: if dry_run { 0 } else { archived },
        skipped_protected,
        ids,
        reason_buckets: sorted_buckets(bucket_counts),
        top_topics: top_topics(topic_counts, 10),
    })
}

fn archive_reason_for(candidate: &GcCandidate) -> &'static str {
    match candidate.gc_kind.as_str() {
        "stale_session_digest" => "stale_session_digest",
        "low_signal" => "low_signal",
        _ => "stale_low_signal",
    }
}

fn protection_reason(candidate: &GcCandidate) -> Option<&'static str> {
    if candidate.polarity.as_deref() == Some("negative") {
        return Some("negative");
    }
    if candidate.apply_when.is_some() {
        return Some("apply_when");
    }
    if candidate.source.as_deref() == Some("user") && candidate.confidence >= 0.95 {
        return Some("high_confidence_user");
    }
    None
}

fn sorted_buckets(counts: HashMap<String, usize>) -> Vec<GcReasonBucket> {
    let mut buckets: Vec<GcReasonBucket> = counts
        .into_iter()
        .map(|(reason, count)| GcReasonBucket { reason, count })
        .collect();
    buckets.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.reason.cmp(&b.reason)));
    buckets
}

fn top_topics(counts: HashMap<String, usize>, limit: usize) -> Vec<GcTopicCount> {
    let mut topics: Vec<GcTopicCount> = counts
        .into_iter()
        .map(|(topic, count)| GcTopicCount { topic, count })
        .collect();
    topics.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.topic.cmp(&b.topic)));
    topics.truncate(limit);
    topics
}


/// Lower stale fact confidence in-place (weekly heart/distillation hook).
pub fn apply_temporal_confidence_decay(conn: &Connection, now_ms: i64) -> Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, confidence, updated_at FROM facts WHERE superseded_by IS NULL AND source != 'user'",
    )?;
    let rows: Vec<(String, f64, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();
    let mut updated = 0usize;
    for (id, confidence, updated_at) in rows {
        let next = crate::memory_decay::decay_stored_confidence(confidence, updated_at, now_ms);
        if (next - confidence).abs() > f64::EPSILON {
            conn.execute(
                "UPDATE facts SET confidence = ?1, updated_at = updated_at WHERE id = ?2",
                params![next, id],
            )?;
            updated += 1;
        }
    }
    Ok(updated)
}
