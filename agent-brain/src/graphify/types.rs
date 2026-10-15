use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphifyRepoRecord {
    pub repo_root: String,
    pub enabled_at: i64,
    #[serde(default)]
    pub last_ingest_at: Option<i64>,
    #[serde(default)]
    pub last_graph_mtime: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphifyJobRecord {
    pub id: String,
    pub repo_root: String,
    pub trigger: String,
    pub mode: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
    pub result_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphifyJobStatus {
    pub job_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

pub use crate::code_nav::{CodeContext, CodeContextNode};

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GraphJson {
    #[serde(default)]
    pub nodes: Vec<GraphJsonNode>,
    #[serde(default)]
    pub links: Vec<GraphJsonEdge>,
    #[serde(default)]
    pub edges: Vec<GraphJsonEdge>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GraphJsonNode {
    pub id: serde_json::Value,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub community: Option<i64>,
    #[serde(default)]
    pub community_id: Option<i64>,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub file_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GraphJsonEdge {
    pub source: serde_json::Value,
    pub target: serde_json::Value,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
    #[serde(default)]
    pub confidence_score: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GraphifyAnalysis {
    #[serde(default)]
    pub gods: Vec<String>,
}

pub(crate) fn node_id_str(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}
