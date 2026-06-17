use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::store::BrainStore;
use crate::types::{RouteTaskResponse, ScoredItem};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalItemLog {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub topic: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainLastContext {
    pub log_id: String,
    pub query_hash: String,
    pub phase: String,
    pub items: Vec<RetrievalItemLog>,
    pub tokens_used: usize,
    pub truncated: bool,
    pub cache_hit: bool,
    pub latency_ms: u64,
}

pub fn query_hash(text: &str) -> String {
    let normalized = text.trim().to_ascii_lowercase();
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub fn log_route(
    store: &BrainStore,
    log_id: &str,
    query: &str,
    phase: &str,
    resp: &RouteTaskResponse,
    scored: &[ScoredItem],
    truncated: bool,
) -> Result<()> {
    let savings = crate::route_briefing::token_savings(resp);
    let (index_total, saved_pct) = savings
        .map(|s| (Some(s.index_total), Some(s.saved_pct.min(100) as u8)))
        .unwrap_or((None, None));

    let items: Vec<RetrievalItemLog> = scored
        .iter()
        .take(20)
        .map(|item| RetrievalItemLog {
            id: item.id.clone(),
            item_type: item.item_type.as_str().to_string(),
            topic: item.topic.clone(),
            score: item.score,
        })
        .collect();
    store.insert_retrieval_log(
        log_id,
        &query_hash(query),
        phase,
        &serde_json::to_string(&items)?,
        resp.tokens_used,
        truncated,
        resp.cache_hit,
        resp.latency_ms,
        index_total,
        saved_pct,
        Some(resp.must_apply.len()),
    )?;
    crate::adoption::record_first_route(store)
}

pub fn log_native_tool_use(
    store: &BrainStore,
    tool_name: &str,
    path: Option<&str>,
    tokens_used: usize,
    tokens_saved: Option<usize>,
    savings_pct: Option<f64>,
    must_apply_active: bool,
    phase: Option<&str>,
    route_log_id: Option<&str>,
) -> Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    store.insert_tool_log(
        &id,
        tool_name,
        path,
        tokens_used,
        tokens_saved,
        savings_pct,
        must_apply_active,
        phase,
        route_log_id,
    )
}

pub fn explain_last(store: &BrainStore, log_id: Option<&str>) -> Result<Option<ExplainLastContext>> {
    let row = match log_id {
        Some(id) => store.get_retrieval_log(id)?,
        None => store.latest_retrieval_log()?,
    };
    let Some(row) = row else {
        return Ok(None);
    };
    let items: Vec<RetrievalItemLog> = serde_json::from_str(&row.items_json)?;
    Ok(Some(ExplainLastContext {
        log_id: row.id,
        query_hash: row.query_hash,
        phase: row.phase,
        items,
        tokens_used: row.tokens_used,
        truncated: row.truncated,
        cache_hit: row.cache_hit,
        latency_ms: row.latency_ms,
    }))
}

pub fn format_inspect_log(row: &crate::db::store::RetrievalLogRow) -> String {
    format!(
        "{}  phase={}  tokens={}  cache={}  {}ms  items={}",
        row.id,
        row.phase,
        row.tokens_used,
        row.cache_hit,
        row.latency_ms,
        row.items_json.len()
    )
}

pub fn log_upstream_call(
    store: &BrainStore,
    log_id: &str,
    server: &str,
    tool: &str,
    tokens_used: usize,
    truncated: bool,
    is_error: bool,
    latency_ms: u64,
) -> Result<()> {
    let items = serde_json::json!([{
        "type": "upstream_call",
        "server": server,
        "tool": tool,
        "is_error": is_error,
    }]);
    store.insert_retrieval_log(
        log_id,
        "upstream",
        "upstream_call",
        &items.to_string(),
        tokens_used,
        truncated,
        false,
        latency_ms,
        None,
        None,
        None,
    )
}
