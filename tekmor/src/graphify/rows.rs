use serde::Serialize;

#[derive(Debug, Clone)]
pub struct CodeGraphNodeRow {
    pub graphify_id: String,
    pub label: String,
    pub community_id: Option<i64>,
    pub is_god_node: bool,
    pub source_file: Option<String>,
    pub file_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AstCodeNodeRow {
    pub repo_root: String,
    pub symbol_name: String,
    pub symbol_kind: String,
    pub content: String,
    pub source_file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub language: String,
    pub doc_comment: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodeGraphEdgeRow {
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub confidence: Option<String>,
    pub confidence_score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AstEdgeRow {
    pub repo_root: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub source_file: String,
    pub start_line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalGraphEdge {
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub source_label: Option<String>,
    pub target_label: Option<String>,
}
