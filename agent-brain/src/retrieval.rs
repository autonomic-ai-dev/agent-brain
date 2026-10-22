//! Query–document matching helpers for hybrid retrieval (BM25 + embeddings).
//!
//! Routing accuracy is the product USP: these functions normalize queries, expand
//! related terms, and score lexical overlap so the right skills surface for any task.

use std::collections::HashSet;

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
    "did", "will", "would", "could", "should", "may", "might", "must", "shall", "can", "need",
    "let", "lets", "this", "that", "these", "those", "it", "its", "we", "you", "i", "me", "my",
    "our", "your", "they", "them", "their", "as", "if", "so", "than", "then", "there", "here",
    "when", "what", "which", "who", "how", "why", "all", "each", "every", "both", "few", "more",
    "most", "other", "some", "such", "no", "not", "only", "own", "same", "too", "very", "just",
    "about", "into", "through", "during", "before", "after", "above", "below", "up", "down", "out",
    "off", "over", "under", "again", "further",
];

/// Related terms that often appear in different wording for the same intent.
const SYNONYM_GROUPS: &[&[&str]] = &[
    &["pr", "pull", "request", "merge", "mergeable"],
    &["review", "audit", "diff", "checklist"],
    &[
        "test", "testing", "tests", "vitest", "jest", "pytest", "mockmvc",
    ],
    &["deploy", "release", "ship", "publish"],
    &["debug", "fix", "bug", "error", "issue", "crash"],
    &["plan", "design", "architect", "roadmap", "spec"],
    &["implement", "build", "add", "create", "write"],
    &["github", "gh", "issue", "pull"],
    &["mcp", "stdio", "transport", "server"],
    &["postgres", "postgresql", "pg", "pooling", "pgbouncer"],
    &["rust", "cargo", "clippy", "anyhow", "thiserror"],
    &[
        "grep",
        "rg",
        "cat",
        "head",
        "tail",
        "token",
        "tokens",
        "efficient",
        "read",
        "file",
        "large",
        "log",
        "dist",
        "artifact",
    ],
    &["branch", "branching", "rebase", "cherry", "pick"],
    &["database", "db", "sql"],
    &["k8s", "kubernetes"],
    &["go", "golang"],
    &["openid", "oidc"],
    &["redis", "caching", "cache"],
    &["csharp", "dotnet", "linq"],
    &["numpy", "tensor", "matrix", "vectorization"],
    &["pandas", "dataframe", "groupby", "aggregations"],
    &["rxjs", "observables", "switchmap", "mergemap"],
];

const SUPERVISOR_TERMS: &[&str] = &[
    "grep",
    "rg",
    "cat",
    "head",
    "tail",
    "token",
    "tokens",
    "efficient",
    "read",
    "file",
    "large",
    "log",
    "dist",
    "artifact",
    "build",
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
    let q_lower = query.to_lowercase();
    if q_lower.contains("c++") {
        out.insert("cpp".to_string());
    }
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

/// Normalize a token for keyword matching (light verb-stem / plural trim).
pub fn normalize_token(token: &str) -> String {
    let t = token.to_lowercase();
    if t.len() > 4 && t.ends_with("ing") {
        return t[..t.len() - 3].to_string();
    }
    if t.len() > 3 && t.ends_with("ed") {
        return t[..t.len() - 2].to_string();
    }
    if t.len() > 2 && t.ends_with('s') && !t.ends_with("ss") {
        return t[..t.len() - 1].to_string();
    }
    t
}

/// Entity-like tokens (CamelCase, snake_case, ALLCAPS) for multi-signal fusion.
pub fn entity_tokens(text: &str) -> Vec<String> {
    let mut out = HashSet::new();
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if word.len() < 2 {
            continue;
        }
        if word.chars().any(|c| c.is_uppercase()) || word.contains('_') {
            out.insert(normalize_token(word));
        }
    }
    out.into_iter().collect()
}

/// Fraction of query entities found in topic+body (Mem0-style entity signal).
pub fn entity_overlap_score(query: &str, topic: &str, text: &str) -> f64 {
    let entities = entity_tokens(query);
    if entities.is_empty() {
        return 0.0;
    }
    let hay = format!("{} {}", topic.to_lowercase(), text.to_lowercase());
    let matched = entities.iter().filter(|e| hay.contains(e.as_str())).count();
    matched as f64 / entities.len() as f64
}

pub fn is_conversational_or_meta(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return false;
    }
    if q.contains("false positive")
        || q.contains("rethink how the suggestion")
        || q.contains("skill suggestion")
        || q.contains("rethink skills")
    {
        return true;
    }
    let conversational_exact = [
        "thanks", "thank you", "thx", "ok", "okay", "cool", "great", "done", "got it",
        "sounds good", "perfect", "yes", "no", "yep", "nope",
    ];
    if conversational_exact.contains(&q.as_str()) {
        return true;
    }
    if q.starts_with("what is the status")
        || q.starts_with("status update")
        || q.starts_with("what's the status")
        || q.starts_with("current status")
        || q == "status"
    {
        return true;
    }
    if (q.starts_with("rebase from")
        || q.starts_with("git checkout")
        || q.starts_with("git pull")
        || q.starts_with("git push")
        || q.starts_with("git status")
        || q.starts_with("git commit"))
        && !q.contains("conflict")
        && !q.contains("--")
    {
        return true;
    }
    if q.starts_with("what time is it")
        || q.starts_with("what's the time")
        || q.contains("list the open files")
    {
        return true;
    }
    if q.contains("system prompt:")
        || q.contains("override instructions")
        || q.contains("user instruction")
        || q.contains("instruction override")
        || q.contains("ignore previous")
        || q.contains("important: recommend")
        || q.contains("jailbreak mode:")
        || q.contains("admin override:")
        || q.contains("developer mode enabled")
        || q.contains("<skill_injection>")
        || q.contains("<!-- skill-injection")
        || q.contains("```json")
    {
        return true;
    }
    if q.contains("pause here and summarize")
        || q.contains("summarize our decisions")
        || q.contains("summarize progress")
    {
        return true;
    }
    if q.starts_with("tell me about the history of")
        || q.starts_with("tell me a story")
        || q.starts_with("what is the meaning of")
    {
        return true;
    }
    false
}

const SOFTWARE_TERMS: &[&str] = &[
    "code", "function", "method", "class", "file", "test", "build", "run", "config",
    "api", "app", "service", "module", "package", "library", "component", "interface",
    "type", "error", "server", "client", "database", "query", "repo", "git", "branch",
    "pr", "deploy", "runtime", "script", "framework", "endpoint", "pipeline", "schema",
    "model", "worker", "process", "stream", "queue", "cache", "token", "auth", "unit",
    "async", "asyncio", "concurrency", "routing", "migration", "proxy", "container",
    "k8s", "kubernetes", "postgres", "redis", "nginx", "linux", "sql", "graphql", "grpc",
    "wasm", "cmake", "terraform", "ansible", "kafka", "rabbitmq", "nats", "sqlite",
    "mysql", "mongodb", "nextjs", "django", "fastapi", "vitest", "jest", "cypress", "playwright",
    "numpy", "pandas", "data", "array", "matrix", "tensor", "vector",
    "pointer", "pointers", "raii", "smart", "mutex", "channel", "goroutine", "goroutines",
    "sync", "synchronization", "microservice", "microservices", "autowired", "injection",
    "maven", "pom", "plugin", "plugins", "dependency", "dependencies", "template", "templates",
    "metaprogramming", "sfinae", "concept", "concepts", "tokio", "mpsc", "cancellation",
    "angular", "rxjs", "pipe", "debouncetime", "rtic", "cortex", "embedded", "interrupt",
    "handler", "handlers", "no_std", "cbindgen", "cdylib", "ffi", "header", "export",
    "exports", "flamegraph", "profiling", "criterion", "microbenchmark", "microbenchmarks",
    "benchmark", "benchmarks", "compile", "compiler", "golang", "typescript", "javascript",
    "csharp", "kotlin", "swift", "ruby", "scala", "elixir", "haskell",
];

pub fn query_has_software_context(query: &str) -> bool {
    let q_lower = query.to_lowercase();
    if q_lower.contains("c++") {
        return true;
    }
    let q_terms = significant_terms(query);
    q_terms.iter().any(|t| {
        let norm = normalize_token(t);
        SOFTWARE_TERMS.contains(&t.as_str()) || SOFTWARE_TERMS.contains(&norm.as_str())
    })
}

pub fn query_forbids_topic(query: &str, topic: &str) -> bool {
    let q = query.to_lowercase();
    let topic_parts: Vec<&str> = topic
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| p.len() >= 2 && !STOPWORDS.contains(p))
        .collect();

    let negative_prefixes = [
        "do not use ",
        "dont use ",
        "don't use ",
        "do not install ",
        "dont install ",
        "don't install ",
        "do not generate ",
        "dont generate ",
        "don't generate ",
        "avoid ",
        "without using ",
        "without ",
        "never use ",
        "refrain from using ",
        "skip ",
    ];

    for prefix in negative_prefixes {
        if let Some(pos) = q.find(prefix) {
            let forbidden_phrase = &q[pos + prefix.len()..];
            for part in &topic_parts {
                if forbidden_phrase.starts_with(part)
                    || forbidden_phrase
                        .split_whitespace()
                        .any(|w| w == *part || w.starts_with(part))
                {
                    return true;
                }
            }
        }
    }
    false
}

pub fn is_informational_query(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.starts_with("why is ")
        || q.starts_with("why did ")
        || q.starts_with("why does ")
        || q.starts_with("how does ")
        || q.starts_with("how do ")
        || q.starts_with("what does ")
        || q.starts_with("what is ")
        || q.starts_with("what are ")
        || q.starts_with("could you explain ")
        || q.starts_with("can you explain ")
        || q.starts_with("explain ")
        || q.starts_with("can we discuss ")
        || q.starts_with("tell me about ")
}

pub fn skill_topic_affinity(query: &str, topic: &str) -> f64 {
    let query_terms = expanded_terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let topic_parts: Vec<String> = topic
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| p.len() >= 2 && !STOPWORDS.contains(p))
        .map(normalize_token)
        .collect();
    if topic_parts.is_empty() {
        return 0.0;
    }
    let matched = topic_parts
        .iter()
        .filter(|tp| {
            query_terms.iter().any(|qt| {
                qt == tp.as_str()
                    || normalize_token(qt) == **tp
                    || (tp.len() >= 4
                        && qt.len() >= 4
                        && (tp.starts_with(qt.as_str()) || qt.starts_with(tp.as_str())))
            })
        })
        .count();

    if topic_parts.len() > 1 && matched == 1 && !query_has_software_context(query) {
        return 0.0;
    }

    matched as f64 / topic_parts.len() as f64
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
    fn entity_overlap_matches_camel_case() {
        let score =
            entity_overlap_score("fix Vitest config", "testing", "Use Vitest for unit tests");
        assert!(score > 0.0);
    }

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
