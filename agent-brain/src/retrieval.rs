//! Query–document matching helpers for hybrid retrieval (BM25 + embeddings).
//!
//! Routing accuracy is the product USP: these functions normalize queries, expand
//! related terms, and score lexical overlap so the right skills surface for any task.

use std::collections::HashSet;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do",
    "does", "did", "will", "would", "could", "should", "may", "might", "must", "shall",
    "can", "need", "let", "lets", "this", "that", "these", "those", "it", "its", "we", "you",
    "i", "me", "my", "our", "your", "they", "them", "their", "as", "if", "so", "than", "then",
    "there", "here", "when", "what", "which", "who", "how", "why", "all", "each", "every",
    "both", "few", "more", "most", "other", "some", "such", "no", "not", "only", "own",
    "same", "too", "very", "just", "about", "into", "through", "during", "before", "after",
    "above", "below", "up", "down", "out", "off", "over", "under", "again", "further",
];

/// Related terms that often appear in different wording for the same intent.
const SYNONYM_GROUPS: &[&[&str]] = &[
    &["pr", "pull", "request", "merge", "mergeable"],
    &["review", "audit", "diff", "checklist"],
    &["test", "testing", "tests", "vitest", "jest", "pytest", "mockmvc"],
    &["deploy", "release", "ship", "publish"],
    &["debug", "fix", "bug", "error", "issue", "crash"],
    &["plan", "design", "architect", "roadmap", "spec"],
    &["implement", "build", "add", "create", "write"],
    &["github", "gh", "issue", "pull"],
    &["mcp", "stdio", "transport", "server"],
    &["postgres", "postgresql", "pg", "pooling", "pgbouncer"],
    &["rust", "cargo", "clippy", "anyhow", "thiserror"],
    &[
        "grep", "rg", "cat", "head", "tail", "token", "tokens", "efficient", "read", "file",
        "large", "log", "dist", "artifact",
    ],
];

const SUPERVISOR_TERMS: &[&str] = &[
    "grep", "rg", "cat", "head", "tail", "token", "tokens", "efficient", "read", "file", "large",
    "log", "dist", "artifact", "build",
];

/// Fraction of supervisor intent terms present in the query (0.0–1.0).
pub fn supervisor_query_strength(query: &str) -> f64 {
    let terms = significant_terms(query);
    if terms.is_empty() {
        return 0.0;
    }
    let matched = terms
        .iter()
        .filter(|t| SUPERVISOR_TERMS.contains(&t.as_str()))
        .count();
    matched as f64 / terms.len() as f64
}

/// Significant terms from a user query (lowercased, stopwords removed, min length 2).
pub fn significant_terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2 && !STOPWORDS.contains(&w))
        .map(str::to_string)
        .collect()
}

/// Query terms plus synonyms from shared intent groups (e.g. `pr` → `pull`, `request`).
pub fn expanded_terms(query: &str) -> Vec<String> {
    let base = significant_terms(query);
    let mut out: HashSet<String> = base.iter().cloned().collect();
    for term in &base {
        for group in SYNONYM_GROUPS {
            if group.iter().any(|g| *g == term.as_str()) {
                for g in *group {
                    out.insert((*g).to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Fraction of query terms (with synonym expansion) found in topic + body (0.0–1.0).
pub fn lexical_overlap_score(query: &str, topic: &str, text: &str) -> f64 {
    let terms = expanded_terms(query);
    if terms.is_empty() {
        return 0.0;
    }
    let hay = format!("{} {}", topic.to_lowercase(), text.to_lowercase());
    let matched = terms.iter().filter(|t| hay.contains(t.as_str())).count();
    matched as f64 / terms.len() as f64
}

/// FTS query preferring precision: AND across significant terms when multiple exist.
pub fn fts_query_strict(query: &str) -> String {
    let terms: Vec<String> = expanded_terms(query)
        .into_iter()
        .filter(|w| w.len() >= 3)
        .map(|w| format!("\"{w}\""))
        .collect();
    match terms.len() {
        0 => {
            let short: Vec<String> = expanded_terms(query)
                .into_iter()
                .map(|w| format!("\"{w}\""))
                .collect();
            if short.is_empty() {
                "\"\"".into()
            } else {
                short.join(" AND ")
            }
        }
        1 => terms[0].clone(),
        _ => terms.join(" AND "),
    }
}

/// FTS query preferring recall: OR across significant terms.
pub fn fts_query_loose(query: &str) -> String {
    let terms: Vec<String> = expanded_terms(query)
        .into_iter()
        .map(|w| format!("\"{w}\""))
        .collect();
    if terms.is_empty() {
        "\"\"".into()
    } else {
        terms.join(" OR ")
    }
}

/// Minimum score for a recommendation to be shown (relative to top non-memory hit).
pub fn minimum_recommendation_score(scored_top_non_memory: f64) -> f64 {
    if scored_top_non_memory <= 0.0 {
        return 0.12;
    }
    (scored_top_non_memory * 0.45).max(0.12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_terms_links_pr_and_pull_request() {
        let terms = expanded_terms("review the PR changes");
        assert!(terms.iter().any(|t| t == "pr"));
        assert!(terms.iter().any(|t| t == "pull"));
    }

    #[test]
    fn lexical_overlap_rewards_matching_description() {
        let query = "review the changes on the PR";
        let good = lexical_overlap_score(
            query,
            "code-review",
            "Dispatch code reviewer for pull request changes before merge",
        );
        let bad = lexical_overlap_score(
            query,
            "cooking-recipes",
            "Pasta sauces and baking tips for weeknight dinners",
        );
        assert!(good > bad, "good={good} bad={bad}");
    }

    #[test]
    fn fts_strict_uses_and_for_multiple_terms() {
        let q = fts_query_strict("review pull request changes");
        assert!(q.contains(" AND "));
    }
}
