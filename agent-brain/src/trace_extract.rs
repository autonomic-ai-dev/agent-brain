//! Mem0-inspired single-pass memory extraction from IDE tool traces (ADD-only).

use anyhow::Result;
use regex::Regex;
use serde::Serialize;

use crate::db::store::{content_hash, word_count, BrainStore, ToolTraceRow};
use crate::embed::Embedder;
use crate::tool_events;

#[derive(Debug, Clone, Serialize)]
pub struct TraceExtractReport {
    pub dry_run: bool,
    pub scanned: usize,
    pub extracted: usize,
    pub skipped: usize,
    pub topics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanations: Vec<TraceExtractExplanation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceExtractExplanation {
    pub trace_id: String,
    pub tool_name: String,
    pub status: TraceExtractStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fact: Option<String>,
    pub why_extracted: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_when: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceExtractStatus {
    Extracted,
    Skipped,
    Duplicate,
}

#[derive(Debug, Clone)]
pub struct TraceExtractConfig {
    pub confidence: f64,
    pub explain: bool,
}

impl Default for TraceExtractConfig {
    fn default() -> Self {
        Self {
            confidence: 0.75,
            explain: false,
        }
    }
}

pub fn explain_pending_traces(store: &BrainStore, home: &std::path::Path) -> Result<Vec<TraceExtractExplanation>> {
    let since_ms = store
        .get_meta("last_trace_extract_ms")?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let _ = tool_events::ingest_hook_events_since(store, home, since_ms)?;
    let rows = store.list_pending_tool_traces(500)?;
    Ok(rows
        .iter()
        .map(|row| explain_trace_row(store, row))
        .collect::<Result<Vec<_>>>()?)
}

pub fn run_trace_extract(
    store: &BrainStore,
    embedder: &Embedder,
    home: &std::path::Path,
    cfg: &TraceExtractConfig,
    dry_run: bool,
) -> Result<TraceExtractReport> {
    let since_ms = store
        .get_meta("last_trace_extract_ms")?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let _ = tool_events::ingest_hook_events_since(store, home, since_ms)?;

    let rows = store.list_pending_tool_traces(500)?;
    let mut extracted = 0usize;
    let mut skipped = 0usize;
    let mut topics = Vec::new();
    let mut explanations = Vec::new();

    for row in &rows {
        let explanation = explain_trace_row(store, row)?;
        if cfg.explain {
            explanations.push(explanation.clone());
        }
        match explanation.status {
            TraceExtractStatus::Extracted => {
                let Some(topic) = explanation.topic.clone() else {
                    skipped += 1;
                    continue;
                };
                let Some(fact) = explanation.fact.clone() else {
                    skipped += 1;
                    continue;
                };
                topics.push(topic.clone());
                if dry_run {
                    extracted += 1;
                    continue;
                }
                let scope_key = row.scope_key.clone().or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .and_then(|c| crate::config::find_repo_root(&c))
                        .map(|p| p.display().to_string())
                });
                let embedding = embedder.embed_one(&format!("{topic} {fact}"))?;
                let hash = content_hash(&fact);
                let res = store.store_fact_full(
                    &topic,
                    &fact,
                    "project",
                    scope_key.as_deref(),
                    cfg.confidence,
                    "trace_extract",
                    &hash,
                    &embedding,
                    "positive",
                    explanation.apply_when.as_deref(),
                    None,
                )?;
                store.insert_fact_lineage(&res.id, "tool_log", &row.id, "extracted_from")?;
                store.mark_trace_extracted(&row.id, Some(&res.id))?;
                extracted += 1;
            }
            TraceExtractStatus::Skipped => {
                skipped += 1;
                if !dry_run && !cfg.explain {
                    store.mark_trace_extracted(&row.id, None)?;
                }
            }
            TraceExtractStatus::Duplicate => {
                skipped += 1;
                if !dry_run && !cfg.explain {
                    store.mark_trace_extracted(&row.id, None)?;
                }
            }
        }
    }

    if !dry_run && !cfg.explain {
        let now = chrono::Utc::now().timestamp_millis();
        store.set_meta("last_trace_extract_ms", &now.to_string())?;
    }

    Ok(TraceExtractReport {
        dry_run,
        scanned: rows.len(),
        extracted,
        skipped,
        topics,
        explanations,
    })
}

fn explain_trace_row(store: &BrainStore, row: &ToolTraceRow) -> Result<TraceExtractExplanation> {
    let detail = row.detail.as_deref().unwrap_or("").trim();
    let path = row.path.as_deref().unwrap_or("").trim();
    let tool = row.tool_name.to_ascii_lowercase();

    if tool.contains("store_memory") {
        return Ok(TraceExtractExplanation {
            trace_id: row.id.clone(),
            tool_name: row.tool_name.clone(),
            status: TraceExtractStatus::Skipped,
            topic: None,
            fact: None,
            why_extracted: "Skipped store_memory traces to avoid feedback loops".into(),
            pattern: None,
            apply_when: None,
        });
    }

    let Some((candidate, pattern, why)) = candidate_from_trace(row) else {
        return Ok(TraceExtractExplanation {
            trace_id: row.id.clone(),
            tool_name: row.tool_name.clone(),
            status: TraceExtractStatus::Skipped,
            topic: None,
            fact: None,
            why_extracted: format!(
                "No extraction pattern matched tool={tool} path={path} detail={detail}"
            ),
            pattern: None,
            apply_when: None,
        });
    };

    if word_count(&candidate.fact) > 50 {
        return Ok(TraceExtractExplanation {
            trace_id: row.id.clone(),
            tool_name: row.tool_name.clone(),
            status: TraceExtractStatus::Skipped,
            topic: Some(candidate.topic.clone()),
            fact: Some(candidate.fact.clone()),
            why_extracted: "Fact exceeds 50-word limit for trace extraction".into(),
            pattern: Some(pattern.into()),
            apply_when: candidate.apply_when.clone(),
        });
    }

    let scope_key = row.scope_key.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|c| crate::config::find_repo_root(&c))
            .map(|p| p.display().to_string())
    });
    let hash = content_hash(&candidate.fact);
    if store.fact_exists_by_hash(&hash, "project", scope_key.as_deref())? {
        return Ok(TraceExtractExplanation {
            trace_id: row.id.clone(),
            tool_name: row.tool_name.clone(),
            status: TraceExtractStatus::Duplicate,
            topic: Some(candidate.topic.clone()),
            fact: Some(candidate.fact.clone()),
            why_extracted: "Duplicate content_hash already stored for this scope".into(),
            pattern: Some(pattern.into()),
            apply_when: candidate.apply_when.clone(),
        });
    }

    Ok(TraceExtractExplanation {
        trace_id: row.id.clone(),
        tool_name: row.tool_name.clone(),
        status: TraceExtractStatus::Extracted,
        topic: Some(candidate.topic.clone()),
        fact: Some(candidate.fact.clone()),
        why_extracted: why.into(),
        pattern: Some(pattern.into()),
        apply_when: candidate.apply_when.clone(),
    })
}

struct TraceCandidate {
    topic: String,
    fact: String,
    apply_when: Option<String>,
}

fn candidate_from_trace(row: &ToolTraceRow) -> Option<(TraceCandidate, &'static str, &'static str)> {
    let detail = row.detail.as_deref().unwrap_or("").trim();
    let path = row.path.as_deref().unwrap_or("").trim();

    if let Some(c) = match_package_manager(detail) {
        return Some((c, "package_manager", "Matched npm/pnpm/yarn/bun install or add command"));
    }
    if let Some(c) = match_pip_install(detail) {
        return Some((c, "pip_install", "Matched pip/pip3 install command"));
    }
    if let Some(c) = match_cargo_add(detail) {
        return Some((c, "cargo_add", "Matched cargo add dependency command"));
    }
    if let Some(c) = match_go_get(detail) {
        return Some((c, "go_get", "Matched go get module command"));
    }
    if let Some(c) = match_brew_install(detail) {
        return Some((c, "brew_install", "Matched brew install formula"));
    }
    if let Some(c) = match_config_edit(path, detail) {
        return Some((c, "config_edit", "Matched known test/build config file edit"));
    }
    if let Some(c) = match_test_runner(detail) {
        return Some((c, "test_runner", "Matched shell trace invoking a test runner"));
    }
    if let Some(c) = match_make_target(detail) {
        return Some((c, "make_target", "Matched make release/test/build target"));
    }
    None
}

fn match_package_manager(detail: &str) -> Option<TraceCandidate> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)(npm|pnpm|yarn|bun)\s+(install|add)(?:\s+-D|\s+--save-dev|\s+--dev)?\s+([\w@./-]+)",
        )
        .expect("regex")
    });
    let caps = re.captures(detail)?;
    let pkg = caps.get(3)?.as_str();
    let topic = slug_topic(pkg);
    Some(TraceCandidate {
        topic: format!("deps-{topic}"),
        fact: format!("Project added dependency {pkg} via package manager"),
        apply_when: Some(r#"["phase:implementing"]"#.into()),
    })
}

fn match_pip_install(detail: &str) -> Option<TraceCandidate> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)(?:pip3?|uv)\s+install(?:\s+-r\s+\S+|\s+)([\w.-]+)").expect("regex")
    });
    let caps = re.captures(detail)?;
    let pkg = caps.get(1)?.as_str();
    if pkg.eq_ignore_ascii_case("requirements.txt") {
        return None;
    }
    Some(TraceCandidate {
        topic: format!("deps-{}", slug_topic(pkg)),
        fact: format!("Project added Python dependency {pkg} via pip/uv"),
        apply_when: Some(r#"["phase:implementing"]"#.into()),
    })
}

fn match_cargo_add(detail: &str) -> Option<TraceCandidate> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)cargo\s+add\s+([\w-]+)").expect("regex"));
    let caps = re.captures(detail)?;
    let crate_name = caps.get(1)?.as_str();
    Some(TraceCandidate {
        topic: format!("deps-{crate_name}"),
        fact: format!("Project added Rust crate {crate_name} via cargo add"),
        apply_when: None,
    })
}

fn match_go_get(detail: &str) -> Option<TraceCandidate> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)go\s+get\s+([\w./@-]+)").expect("regex"));
    let caps = re.captures(detail)?;
    let module = caps.get(1)?.as_str();
    Some(TraceCandidate {
        topic: format!("deps-{}", slug_topic(module)),
        fact: format!("Project added Go module {module} via go get"),
        apply_when: None,
    })
}

fn match_brew_install(detail: &str) -> Option<TraceCandidate> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)brew\s+install\s+([\w@/-]+)").expect("regex"));
    let caps = re.captures(detail)?;
    let formula = caps.get(1)?.as_str();
    Some(TraceCandidate {
        topic: format!("tool-{}", slug_topic(formula)),
        fact: format!("Developer installed {formula} via Homebrew"),
        apply_when: None,
    })
}

fn match_config_edit(path: &str, detail: &str) -> Option<TraceCandidate> {
    let lower = path.to_ascii_lowercase();
    if lower.contains("vitest.config") || lower.contains("vite.config") {
        return Some(TraceCandidate {
            topic: "testing-stack".into(),
            fact: "Vitest configuration was edited in this project".into(),
            apply_when: Some(r#"["phase:implementing"]"#.into()),
        });
    }
    if lower.ends_with("package.json") && (detail.contains("vitest") || detail.contains("jest")) {
        let runner = if detail.to_ascii_lowercase().contains("vitest") {
            "Vitest"
        } else {
            "Jest"
        };
        return Some(TraceCandidate {
            topic: "testing-stack".into(),
            fact: format!("package.json change references {runner} for tests"),
            apply_when: Some(r#"["phase:implementing"]"#.into()),
        });
    }
    if lower.ends_with("cargo.toml") && detail.contains("agent-brain") {
        return Some(TraceCandidate {
            topic: "rust-workspace".into(),
            fact: "Cargo.toml references agent-brain crate dependencies".into(),
            apply_when: Some(r#"["phase:implementing"]"#.into()),
        });
    }
    None
}

fn match_test_runner(detail: &str) -> Option<TraceCandidate> {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("vitest") && (lower.contains("test") || lower.contains("run")) {
        return Some(TraceCandidate {
            topic: "testing-stack".into(),
            fact: "Shell trace shows Vitest used for running tests".into(),
            apply_when: Some(r#"["phase:implementing"]"#.into()),
        });
    }
    if lower.contains("cargo test") {
        return Some(TraceCandidate {
            topic: "testing-stack".into(),
            fact: "Shell trace shows cargo test used for Rust verification".into(),
            apply_when: Some(r#"["phase:implementing"]"#.into()),
        });
    }
    None
}

fn match_make_target(detail: &str) -> Option<TraceCandidate> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)make\s+(release-macos|release|test|proofs)").expect("regex"));
    let caps = re.captures(detail)?;
    let target = caps.get(1)?.as_str();
    Some(TraceCandidate {
        topic: "build-targets".into(),
        fact: format!("Shell trace shows make {target} used in this repo"),
        apply_when: Some(r#"["phase:implementing"]"#.into()),
    })
}

fn slug_topic(raw: &str) -> String {
    raw.split('@').next().unwrap_or(raw).replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_npm_install() {
        let row = ToolTraceRow {
            id: "1".into(),
            tool_name: "Shell".into(),
            path: None,
            detail: Some("npm install -D vitest".into()),
            scope_key: None,
        };
        let (c, pattern, _) = candidate_from_trace(&row).unwrap();
        assert_eq!(c.topic, "deps-vitest");
        assert_eq!(pattern, "package_manager");
    }

    #[test]
    fn extracts_from_cargo_add() {
        let row = ToolTraceRow {
            id: "2".into(),
            tool_name: "Shell".into(),
            path: None,
            detail: Some("cargo add anyhow".into()),
            scope_key: None,
        };
        let (c, pattern, _) = candidate_from_trace(&row).unwrap();
        assert_eq!(c.topic, "deps-anyhow");
        assert_eq!(pattern, "cargo_add");
    }

    #[test]
    fn extracts_from_pip_install() {
        let row = ToolTraceRow {
            id: "3".into(),
            tool_name: "Shell".into(),
            path: None,
            detail: Some("pip install pydantic".into()),
            scope_key: None,
        };
        let (c, pattern, _) = candidate_from_trace(&row).unwrap();
        assert_eq!(c.topic, "deps-pydantic");
        assert_eq!(pattern, "pip_install");
    }

    #[test]
    fn explain_skips_store_memory() {
        let row = ToolTraceRow {
            id: "4".into(),
            tool_name: "store_memory".into(),
            path: None,
            detail: Some("topic=x".into()),
            scope_key: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("brain.db");
        let store = BrainStore::open(&db).unwrap();
        let ex = explain_trace_row(&store, &row).unwrap();
        assert_eq!(ex.status, TraceExtractStatus::Skipped);
        assert!(ex.why_extracted.contains("feedback"));
    }
}
