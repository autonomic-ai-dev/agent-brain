use anyhow::Result;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

#[derive(Debug, Default, Serialize)]
pub struct GcStats {
    pub facts_deduped: u64,
    pub facts_removed: u64,
    pub index_items_deduped: u64,
    pub index_items_removed: u64,
    pub bytes_reclaimed: u64,
}

/// MinHash signature length (O(n×128) vs O(n²) pairwise).
pub const MINHASH_SIZE: usize = 128;
/// LSH bands: band match ⇒ candidate pair.
const LSH_BANDS: usize = 16;
const LSH_ROWS: usize = MINHASH_SIZE / LSH_BANDS;

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
        if group.len() < 2 {
            continue;
        }
        let sigs: Vec<[u64; MINHASH_SIZE]> = group
            .iter()
            .map(|row| {
                let text = row.get("fact").and_then(|v| v.as_str()).unwrap_or("");
                minhash_signature(text)
            })
            .collect();
        let pairs = lsh_candidate_pairs(&sigs);
        let mut invalidated: HashSet<usize> = HashSet::new();
        for (i, j) in pairs {
            if invalidated.contains(&i) || invalidated.contains(&j) {
                continue;
            }
            // Exact or near-duplicate via MinHash agreement.
            if sigs[i] == sigs[j] || minhash_jaccard(&sigs[i], &sigs[j]) >= 0.95 {
                let b_id = group[j].get("id").and_then(|v| v.as_str()).unwrap_or("");
                if !b_id.is_empty() {
                    let _ = store.invalidate_fact(b_id);
                    invalidated.insert(j);
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
        if indices.len() < 2 {
            continue;
        }
        // Prefer embedding MinHash when available; fall back to text.
        let sigs: Vec<[u64; MINHASH_SIZE]> = indices
            .iter()
            .map(|&idx| {
                let item = &items[idx];
                if let Some(ref bytes) = item.embedding {
                    let emb = crate::db::store::bytes_to_f32(bytes);
                    minhash_embedding(&emb)
                } else {
                    minhash_signature(&item.text)
                }
            })
            .collect();
        let pairs = lsh_candidate_pairs(&sigs);
        let mut deleted: HashSet<usize> = HashSet::new();
        for (i, j) in pairs {
            if deleted.contains(&i) || deleted.contains(&j) {
                continue;
            }
            let a = &items[indices[i]];
            let b = &items[indices[j]];
            let near = if let (Some(ref a_bytes), Some(ref b_bytes)) = (&a.embedding, &b.embedding)
            {
                let a_emb = crate::db::store::bytes_to_f32(a_bytes);
                let b_emb = crate::db::store::bytes_to_f32(b_bytes);
                crate::embed::cosine(&a_emb, &b_emb) > 0.95
            } else {
                minhash_jaccard(&sigs[i], &sigs[j]) >= 0.95
            };
            if near {
                let _ = store.delete_indexed_items_under_prefix(&b.source_path);
                deleted.insert(j);
                count += 1;
            }
        }
    }
    Ok(count)
}

/// MinHash over whitespace tokens (128 independent hash functions).
pub fn minhash_signature(text: &str) -> [u64; MINHASH_SIZE] {
    let tokens: Vec<&str> = text.split_whitespace().filter(|t| !t.is_empty()).collect();
    let mut sig = [u64::MAX; MINHASH_SIZE];
    if tokens.is_empty() {
        return sig;
    }
    for (seed, slot) in sig.iter_mut().enumerate() {
        let mut best = u64::MAX;
        for token in &tokens {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            seed.hash(&mut hasher);
            token.hash(&mut hasher);
            best = best.min(hasher.finish());
        }
        *slot = best;
    }
    sig
}

/// MinHash over quantized embedding buckets (stable near-dup detector).
fn minhash_embedding(emb: &[f32]) -> [u64; MINHASH_SIZE] {
    // Bucket each dim into 16 levels so near-identical vectors share tokens.
    let tokens: Vec<String> = emb
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let bucket = ((v + 1.0) * 8.0).clamp(0.0, 15.0) as u8;
            format!("{i}:{bucket}")
        })
        .collect();
    let mut sig = [u64::MAX; MINHASH_SIZE];
    for (seed, slot) in sig.iter_mut().enumerate() {
        let mut best = u64::MAX;
        for token in &tokens {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            seed.hash(&mut hasher);
            token.hash(&mut hasher);
            best = best.min(hasher.finish());
        }
        *slot = best;
    }
    sig
}

pub fn minhash_jaccard(a: &[u64; MINHASH_SIZE], b: &[u64; MINHASH_SIZE]) -> f64 {
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    matches as f64 / MINHASH_SIZE as f64
}

/// LSH banding: emit candidate index pairs that share at least one band.
pub fn lsh_candidate_pairs(sigs: &[[u64; MINHASH_SIZE]]) -> Vec<(usize, usize)> {
    let mut buckets: HashMap<(usize, u64), Vec<usize>> = HashMap::new();
    for (idx, sig) in sigs.iter().enumerate() {
        for band in 0..LSH_BANDS {
            let start = band * LSH_ROWS;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for r in 0..LSH_ROWS {
                sig[start + r].hash(&mut hasher);
            }
            buckets
                .entry((band, hasher.finish()))
                .or_default()
                .push(idx);
        }
    }
    let mut pairs: HashSet<(usize, usize)> = HashSet::new();
    for members in buckets.values() {
        if members.len() < 2 {
            continue;
        }
        for i in 0..members.len() {
            for j in (i + 1)..members.len() {
                let a = members[i].min(members[j]);
                let b = members[i].max(members[j]);
                pairs.insert((a, b));
            }
        }
    }
    pairs.into_iter().collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minhash_identical_texts_agree() {
        let a = minhash_signature("ReadyQueue critical path scheduler");
        let b = minhash_signature("ReadyQueue critical path scheduler");
        assert_eq!(a, b);
        assert!((minhash_jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn lsh_finds_duplicate_pair() {
        let sigs = vec![
            minhash_signature("alpha beta gamma"),
            minhash_signature("completely different text here"),
            minhash_signature("alpha beta gamma"),
        ];
        let pairs = lsh_candidate_pairs(&sigs);
        assert!(
            pairs.contains(&(0, 2)),
            "expected (0,2) in {pairs:?}"
        );
    }
}
