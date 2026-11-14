use tekmor::graphify::{CodeGraphEdgeRow, CodeGraphNodeRow};
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use tekmor::db::store::BrainStore;

use crate::repos::touch_ingest;
use crate::types::{node_id_str, GraphJson, GraphJsonEdge, GraphJsonNode, GraphifyAnalysis};

pub fn ingest_repo(store: &BrainStore, home: &Path, repo_root: &Path) -> Result<IngestReport> {
    let report = ingest_graph_at_path(store, repo_root)?;
    let graph_path = repo_root.join("graphify-out").join("graph.json");
    let graph_mtime = graph_path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    touch_ingest(home, repo_root, graph_mtime)?;
    Ok(report)
}

pub fn ingest_graph_at_path(store: &BrainStore, repo_root: &Path) -> Result<IngestReport> {
    let graph_path = repo_root.join("graphify-out").join("graph.json");
    if !graph_path.is_file() {
        bail!(
            "no graph at {} — run graphify in the repo first",
            graph_path.display()
        );
    }
    let raw = fs::read_to_string(&graph_path)
        .with_context(|| format!("read {}", graph_path.display()))?;
    let graph: GraphJson = serde_json::from_str(&raw).context("parse graph.json")?;
    let gods = load_god_nodes(repo_root);
    let now = chrono::Utc::now().timestamp();
    let repo_str = repo_root.display().to_string();

    let nodes = normalize_nodes(&graph.nodes, &gods);
    let edges = normalize_edges(&graph.links, &graph.edges);

    store.replace_code_graph(&repo_str, &nodes, &edges, now)?;
    store.bump_index_version()?;

    Ok(IngestReport {
        nodes: nodes.len(),
        edges: edges.len(),
        god_nodes: gods.len(),
    })
}

#[derive(Debug, Clone)]
pub struct IngestReport {
    pub nodes: usize,
    pub edges: usize,
    pub god_nodes: usize,
}

fn load_god_nodes(repo_root: &Path) -> Vec<String> {
    let path = repo_root
        .join("graphify-out")
        .join(".graphify_analysis.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<GraphifyAnalysis>(&raw)
        .map(|a| a.gods)
        .unwrap_or_default()
}

fn normalize_nodes(nodes: &[GraphJsonNode], gods: &[String]) -> Vec<CodeGraphNodeRow> {
    nodes
        .iter()
        .map(|n| {
            let graphify_id = node_id_str(&n.id);
            let label = n.label.clone().unwrap_or_else(|| graphify_id.clone());
            let community_id = n.community.or(n.community_id);
            let is_god_node = gods.iter().any(|g| g == &label || g == &graphify_id);
            CodeGraphNodeRow {
                graphify_id,
                label,
                community_id,
                is_god_node,
                source_file: n.source_file.clone(),
                file_type: n.file_type.clone(),
            }
        })
        .collect()
}

fn normalize_edges(links: &[GraphJsonEdge], edges: &[GraphJsonEdge]) -> Vec<CodeGraphEdgeRow> {
    let mut out = Vec::new();
    for e in links.iter().chain(edges.iter()) {
        out.push(edge_row(e));
    }
    out
}

fn edge_row(e: &GraphJsonEdge) -> CodeGraphEdgeRow {
    CodeGraphEdgeRow {
        source_id: node_id_str(&e.source),
        target_id: node_id_str(&e.target),
        relation: e
            .relation
            .clone()
            .or_else(|| e.key.clone())
            .unwrap_or_else(|| "related".into()),
        confidence: e.confidence.clone(),
        confidence_score: e.confidence_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_networkx_style_graph() {
        let raw = r#"{
            "nodes": [{"id": "Auth", "label": "AuthModule"}],
            "links": [{"source": "Auth", "target": "Db", "relation": "calls"}]
        }"#;
        let g: GraphJson = serde_json::from_str(raw).unwrap();
        let nodes = normalize_nodes(&g.nodes, &["AuthModule".into()]);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].is_god_node);
        let edges = normalize_edges(&g.links, &g.edges);
        assert_eq!(edges[0].relation, "calls");
    }
}
