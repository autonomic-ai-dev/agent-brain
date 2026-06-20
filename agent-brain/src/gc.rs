use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Default, Serialize)]
pub struct GcStats {
    pub facts_deduped: u64,
    pub facts_removed: u64,
    pub index_items_deduped: u64,
    pub index_items_removed: u64,
    pub bytes_reclaimed: u64,
}

pub fn run_gc(
    store: &crate::db::store::BrainStore,
    min_confidence: f64,
    _max_age_days: u64,
) -> Result<GcStats> {
    let mut stats = GcStats::default();

    stats.facts_deduped = dedup_facts(store)?;
    stats.index_items_deduped = dedup_indexed_items(store)?;
    stats.facts_removed = prune_low_confidence(store, min_confidence)?;
    stats.index_items_removed = prune_stale_index(store)?;

    let before = db_file_size(store)?;
    store.checkpoint_wal()?;
    let after = db_file_size(store)?;
    stats.bytes_reclaimed = before.saturating_sub(after);

    Ok(stats)
}

fn dedup_facts(store: &crate::db::store::BrainStore) -> Result<u64> {
    let rows = store.list_facts(10_000)?;
    let mut by_topic: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for row in &rows {
        let topic = row
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        by_topic.entry(topic).or_default().push(row.clone());
    }

    let mut count = 0u64;
    for (_topic, group) in &by_topic {
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let a_text = group[i].get("fact").and_then(|v| v.as_str()).unwrap_or("");
                let b_text = group[j].get("fact").and_then(|v| v.as_str()).unwrap_or("");
                let a_id = group[i].get("id").and_then(|v| v.as_str()).unwrap_or("");
                let b_id = group[j].get("id").and_then(|v| v.as_str()).unwrap_or("");
                if a_text == b_text && !a_id.is_empty() && !b_id.is_empty() {
                    let _ = store.invalidate_fact(b_id);
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn dedup_indexed_items(store: &crate::db::store::BrainStore) -> Result<u64> {
    let items = store.load_searchable_items()?;
    let mut topic_groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, item) in items.iter().enumerate() {
        topic_groups
            .entry(item.topic.clone())
            .or_default()
            .push(idx);
    }

    let mut count = 0u64;
    for (_topic, indices) in &topic_groups {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = &items[indices[i]];
                let b = &items[indices[j]];
                if let (Some(ref a_bytes), Some(ref b_bytes)) = (&a.embedding, &b.embedding) {
                    let a_emb = crate::db::store::bytes_to_f32(a_bytes);
                    let b_emb = crate::db::store::bytes_to_f32(b_bytes);
                    if crate::embed::cosine(&a_emb, &b_emb) > 0.95 {
                        let _ = store.delete_indexed_items_under_prefix(&b.source_path);
                        count += 1;
                    }
                }
            }
        }
    }
    Ok(count)
}

fn prune_low_confidence(store: &crate::db::store::BrainStore, min_confidence: f64) -> Result<u64> {
    let rows = store.list_facts(10_000)?;
    let mut count = 0u64;
    for row in &rows {
        let conf = row
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if conf < min_confidence && !id.is_empty() {
            let _ = store.invalidate_fact(id);
            count += 1;
        }
    }
    Ok(count)
}

fn prune_stale_index(store: &crate::db::store::BrainStore) -> Result<u64> {
    let items = store.load_searchable_items()?;
    let mut count = 0u64;
    for item in &items {
        if !std::path::Path::new(&item.source_path).exists() {
            let _ = store.delete_indexed_items_under_prefix(&item.source_path);
            count += 1;
        }
    }
    Ok(count)
}

fn db_file_size(store: &crate::db::store::BrainStore) -> Result<u64> {
    let db_path: String =
        store.with_conn(|c| Ok(c.query_row("PRAGMA database_list", [], |row| row.get(2))?))?;
    Ok(std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0))
}
