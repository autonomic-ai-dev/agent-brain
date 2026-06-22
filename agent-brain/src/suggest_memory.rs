//! Promote hook-captured memory suggestions into durable store.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::db::write_queue::store_memory_payload;
use crate::db::{send_and_recv, WriteOp};
use crate::engine::Engine;
use crate::route_briefing;
use crate::token_tools::BLOCKED_SEGMENTS;

#[derive(Debug, Clone, Serialize)]
pub struct ApproveReport {
    pub topic: String,
    pub fact: String,
    pub polarity: String,
    pub stored: bool,
    pub deduplicated: bool,
    pub id: Option<String>,
    pub apply_when: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone)]
struct ParsedSuggestion {
    topic: String,
    fact: String,
    polarity: String,
    path: Option<String>,
    source: &'static str,
}

pub fn approve_pending(engine: &Engine) -> Result<ApproveReport> {
    let home = &engine.config.home;
    let (raw, source) = if let Some(v) = route_briefing::read_anti_pattern_suggestion(home) {
        (v, "anti_pattern")
    } else if let Some(v) = route_briefing::read_edit_memory_suggestion(home) {
        (v, "edit")
    } else {
        bail!("no memory suggestion pending — hooks stage one after Read steer or file edits");
    };
    let parsed = parse_suggestion(&raw, source)?;
    let apply_when = apply_when_for_path(parsed.path.as_deref());
    let scope_key = std::env::current_dir()
        .ok()
        .and_then(|c| crate::config::find_repo_root(&c))
        .map(|p| p.display().to_string());

    let value = send_and_recv(engine.write_queue(), |resp_tx| WriteOp::StoreMemory {
        resp_tx,
        payload: store_memory_payload::StoreMemoryRequest {
            topic: parsed.topic.clone(),
            fact: parsed.fact.clone(),
            scope: "project".into(),
            scope_key,
            confidence: if parsed.source == "edit" { 0.75 } else { 0.95 },
            polarity: Some(parsed.polarity.clone()),
            apply_when: if apply_when.is_empty() {
                None
            } else {
                Some(apply_when.clone())
            },
            valid_from: None,
            invalid_at: None,
        },
    })?;

    clear_suggestion(home, parsed.source)?;

    Ok(ApproveReport {
        topic: parsed.topic,
        fact: parsed.fact,
        polarity: parsed.polarity,
        stored: value
            .get("stored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        deduplicated: value
            .get("deduplicated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        apply_when,
        source: Some(parsed.source.to_string()),
    })
}

pub fn reject_pending(home: &Path) -> Result<()> {
    if route_briefing::read_anti_pattern_suggestion(home).is_some() {
        route_briefing::clear_anti_pattern_suggestion(home)?;
        return Ok(());
    }
    if route_briefing::read_edit_memory_suggestion(home).is_some() {
        route_briefing::clear_edit_memory_suggestion(home)?;
        return Ok(());
    }
    bail!("no memory suggestion pending");
}

fn clear_suggestion(home: &Path, source: &str) -> Result<()> {
    match source {
        "anti_pattern" => route_briefing::clear_anti_pattern_suggestion(home),
        "edit" => route_briefing::clear_edit_memory_suggestion(home),
        _ => Ok(()),
    }
}

fn parse_suggestion(raw: &Value, source: &'static str) -> Result<ParsedSuggestion> {
    let topic = raw
        .get("topic")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .context("suggestion missing topic")?
        .to_string();
    let fact = raw
        .get("fact")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .context("suggestion missing fact")?
        .to_string();
    let polarity = raw
        .get("polarity")
        .and_then(|v| v.as_str())
        .unwrap_or(if source == "edit" { "positive" } else { "negative" })
        .to_string();
    let path = raw.get("path").and_then(|v| v.as_str()).map(str::to_string);
    Ok(ParsedSuggestion {
        topic,
        fact,
        polarity,
        path,
        source,
    })
}

pub fn apply_when_for_path(path: Option<&str>) -> Vec<String> {
    let Some(path) = path else {
        return Vec::new();
    };
    let lower = path.replace('\\', "/").to_lowercase();
    for segment in BLOCKED_SEGMENTS {
        if lower.contains(segment) {
            return vec![format!("path:**/{segment}/**")];
        }
    }
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|name| vec![format!("path:**/{name}")])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_when_for_dist_path() {
        let when = apply_when_for_path(Some("/app/dist/bundle.js"));
        assert_eq!(when, vec!["path:**/dist/**"]);
    }

    #[test]
    fn apply_when_for_named_file() {
        let when = apply_when_for_path(Some("/tmp/large.log"));
        assert_eq!(when, vec!["path:**/large.log"]);
    }
}
