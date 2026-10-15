//! Lightweight code-navigation DTOs always available on route_task responses.
//! Heavy graphify orchestration lives behind `--features extras`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeContextNode {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeContext {
    pub god_nodes: Vec<String>,
    pub relevant_nodes: Vec<CodeContextNode>,
    pub graph_stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ingested_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScratchpadEntry {
    pub id: String,
    pub repo_root: String,
    pub agent_id: Option<String>,
    pub content: String,
    pub context_type: String,
    pub tags: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolMatch {
    pub label: String,
    pub source_file: Option<String>,
    pub file_type: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub ast_symbol: Option<String>,
}
