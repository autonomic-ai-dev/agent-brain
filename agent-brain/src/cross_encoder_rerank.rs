//! Cross-encoder-style rerank of top-K candidates (lexical interaction, no LLM).

pub const DEFAULT_TOP_K: usize = 20;

/// Blend hybrid retrieval score with lexical cross-score so rerank refines, not replaces.
const BASE_SCORE_WEIGHT: f64 = 0.88;
const CROSS_SCORE_WEIGHT: f64 = 0.12;

#[must_use]
pub fn cross_score(query: &str, topic: &str, text: &str) -> f64 {
    let q = query.to_lowercase();
    let doc = format!("{topic} {text}").to_lowercase();
    if q.is_empty() || doc.is_empty() {
        return 0.0;
    }
    let q_tokens: Vec<&str> = q.split_whitespace().filter(|t| t.len() > 2).collect();
    if q_tokens.is_empty() {
        return 0.0;
    }
    let hit = q_tokens.iter().filter(|t| doc.contains(*t)).count();
    let token_recall = hit as f64 / q_tokens.len() as f64;
    let q_words: Vec<&str> = q.split_whitespace().collect();
    let bigram_hits = q_words
        .windows(2)
        .filter(|w| doc.contains(&format!("{} {}", w[0], w[1])))
        .count();
    let bigram = if q_words.len() >= 2 {
        bigram_hits as f64 / (q_words.len() - 1) as f64
    } else {
        0.0
    };
    (0.65 * token_recall + 0.35 * bigram).clamp(0.0, 1.0)
}

#[must_use]
fn combined_rerank_score(base_score: f64, base_max: f64, cross: f64) -> f64 {
    let normalized_base = if base_max > 1e-9 {
        base_score / base_max
    } else {
        0.0
    };
    BASE_SCORE_WEIGHT * normalized_base + CROSS_SCORE_WEIGHT * cross
}

pub fn rerank_scored_items(query: &str, scored: &mut [crate::types::ScoredItem], top_k: usize) {
    if scored.len() <= 1 {
        return;
    }
    let k = top_k.min(scored.len());
    let base_max = scored[..k]
        .iter()
        .map(|item| item.score)
        .fold(0.0_f64, f64::max);
    let mut head: Vec<(f64, crate::types::ScoredItem)> = scored[..k]
        .iter()
        .cloned()
        .map(|item| {
            let cross = cross_score(query, &item.topic, &item.text);
            let combined = combined_rerank_score(item.score, base_max, cross);
            (combined, item)
        })
        .collect();
    head.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (i, (_, item)) in head.into_iter().enumerate() {
        scored[i] = item;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_score_prefers_overlap() {
        let strong = cross_score("token budget wasm", "agent-heart", "WASM fuel token budget");
        let weak = cross_score("token budget wasm", "other", "unrelated topic");
        assert!(strong > weak);
    }

    #[test]
    fn rerank_preserves_strong_hybrid_leader() {
        use crate::types::{ItemType, ScoredItem};

        let mut scored = vec![
            ScoredItem {
                id: "a".into(),
                item_type: ItemType::Skill,
                topic: "react-perf".into(),
                text: "memoization patterns".into(),
                source_path: None,
                scope: "global".into(),
                score: 10.0,
                polarity: None,
                apply_when_matched: false,
            },
            ScoredItem {
                id: "b".into(),
                item_type: ItemType::Skill,
                topic: "unrelated".into(),
                text: "token budget wasm fuel".into(),
                source_path: None,
                scope: "global".into(),
                score: 5.0,
                polarity: None,
                apply_when_matched: false,
            },
        ];
        rerank_scored_items("react memo performance", &mut scored, 20);
        assert_eq!(scored[0].id, "a");
    }
}
