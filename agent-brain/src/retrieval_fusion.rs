//! Reciprocal Rank Fusion (RRF) for multi-source retrieval.
//!
//! Merges ranked lists (BM25, HNSW/ANN, …) with:
//! `score(d) = Σ 1 / (k + rank_i(d))` where rank is 1-indexed.

use std::collections::HashMap;

/// Default RRF constant (`k = 60` per Cormack et al.).
pub const RRF_K: f64 = 60.0;

/// Fuse one or more ranked lists (best-first). Each entry is `(id, source_score)`.
/// Source scores are ignored; only rank order matters.
pub fn rrf_fuse(rankings: &[&[(String, f64)]], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for ranking in rankings {
        for (rank, (id, _)) in ranking.iter().enumerate() {
            *scores.entry(id.clone()).or_default() += 1.0 / (k + (rank + 1) as f64);
        }
    }
    let mut out: Vec<(String, f64)> = scores.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Normalize RRF scores to `[0, 1]` by dividing by the max score (or 0 if empty).
pub fn rrf_normalize(fused: &[(String, f64)]) -> HashMap<String, f64> {
    let max = fused
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0f64, f64::max);
    if max <= 0.0 {
        return HashMap::new();
    }
    fused
        .iter()
        .map(|(id, s)| (id.clone(), s / max))
        .collect()
}

/// True when `topic` is an exact symbol-like token present in `query` (AST boost gate).
/// Match is case-sensitive so `ReadyQueue` ≠ `readyqueue`.
pub fn exact_symbol_match(query: &str, topic: &str) -> bool {
    let topic = topic.trim();
    if topic.is_empty() || topic.contains(' ') {
        return false;
    }
    // Require identifier shape (CamelCase, snake_case, or ALLCAPS-ish).
    let looks_like_symbol = topic.contains('_')
        || topic.chars().any(|c| c.is_ascii_uppercase())
        || topic.chars().all(|c| c.is_ascii_alphanumeric());
    if !looks_like_symbol {
        return false;
    }
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .any(|w| w == topic)
}

/// Multiplicative boost applied when [`exact_symbol_match`] is true.
pub const AST_SYMBOL_BOOST: f64 = 1.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_beats_single_source() {
        // Doc A ranks #1 in BM25 but #3 in ANN; B is #1 ANN only.
        // RRF should prefer A (present in both lists) over B.
        let bm25 = vec![
            ("a".into(), 10.0),
            ("c".into(), 5.0),
            ("d".into(), 1.0),
        ];
        let ann = vec![
            ("b".into(), 0.99),
            ("c".into(), 0.8),
            ("a".into(), 0.7),
        ];
        let fused = rrf_fuse(&[&bm25, &ann], RRF_K);
        assert_eq!(fused[0].0, "a", "dual-listed doc should win RRF: {fused:?}");
        let single = rrf_fuse(&[&ann], RRF_K);
        assert_eq!(single[0].0, "b");
        // Multi-source score for `a` exceeds single-source score for `b`.
        let a_multi = fused.iter().find(|(id, _)| id == "a").unwrap().1;
        let b_single = single.iter().find(|(id, _)| id == "b").unwrap().1;
        assert!(
            a_multi > b_single,
            "rrf multi-source ({a_multi}) should beat single-source top ({b_single})"
        );
    }

    #[test]
    fn exact_symbol_match_boosts_identifiers() {
        assert!(exact_symbol_match("fix ReadyQueue scheduler", "ReadyQueue"));
        assert!(!exact_symbol_match("fix the scheduler", "ReadyQueue"));
        assert!(!exact_symbol_match("ReadyQueue", "ready queue"));
    }
}
