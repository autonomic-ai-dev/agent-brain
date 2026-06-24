use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use uuid::Uuid;

use crate::db::store::BrainStore;

use super::repos::touch_ingest;
use super::types::{node_id_str, GraphJson, GraphJsonEdge, GraphJsonNode, GraphifyAnalysis};

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

/// Parse `graphify-out/graph.json` and upsert into `brain.db` (no repos.json update).
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

impl BrainStore {
    /// Upsert AST-derived symbol nodes into code_graph_nodes.
    /// Uses INSERT OR REPLACE keyed on (repo_root, graphify_id) so graphify nodes are not disturbed.
    pub fn upsert_ast_code_nodes(
        &self,
        repo_root: &str,
        nodes: &[AstCodeNodeRow],
        ingested_at: i64,
    ) -> Result<()> {
        self.with_conn(|conn| {
            for n in nodes {
                let graphify_id = format!("ast:{}:{}", n.source_file, n.symbol_name);
                let id = format!("{repo_root}:{graphify_id}");
                let label = format!("{}.{}", n.language, n.symbol_name);
                let file_type = format!("ast:{}", n.language);
                let ast_json = serde_json::json!({
                    "kind": n.symbol_kind,
                    "content": n.content,
                    "doc": n.doc_comment,
                });
                conn.execute(
                    r#"
                    INSERT INTO code_graph_nodes (
                        id, repo_root, graphify_id, label, community_id, is_god_node,
                        source_file, file_type, ingested_at, ast_symbol, start_line, end_line
                    ) VALUES (?1, ?2, ?3, ?4, NULL, 0, ?5, ?6, ?7, ?8, ?9, ?10)
                    ON CONFLICT(repo_root, graphify_id) DO UPDATE SET
                        label = excluded.label,
                        source_file = excluded.source_file,
                        file_type = excluded.file_type,
                        ingested_at = excluded.ingested_at,
                        ast_symbol = excluded.ast_symbol,
                        start_line = excluded.start_line,
                        end_line = excluded.end_line
                    "#,
                    rusqlite::params![
                        id,
                        repo_root,
                        graphify_id,
                        label,
                        n.source_file,
                        file_type,
                        ingested_at,
                        ast_json.to_string(),
                        n.start_line as i64,
                        n.end_line as i64,
                    ],
                )?;
            }
            Ok(())
        })
    }

    /// Atomically replace ALL code graph nodes and edges for a repo (graphify pipeline).
    /// NOTE: this deletes AST-derived nodes too. The AST pipeline re-adds them on the next sync_index().
    pub fn replace_code_graph(
        &self,
        repo_root: &str,
        nodes: &[CodeGraphNodeRow],
        edges: &[CodeGraphEdgeRow],
        ingested_at: i64,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM code_graph_edges WHERE repo_root = ?1",
                [repo_root],
            )?;
            conn.execute(
                "DELETE FROM code_graph_nodes WHERE repo_root = ?1",
                [repo_root],
            )?;
            for n in nodes {
                let id = format!("{repo_root}:{}", n.graphify_id);
                conn.execute(
                    r#"
                    INSERT INTO code_graph_nodes (
                        id, repo_root, graphify_id, label, community_id, is_god_node,
                        source_file, file_type, ingested_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    "#,
                    rusqlite::params![
                        id,
                        repo_root,
                        n.graphify_id,
                        n.label,
                        n.community_id,
                        n.is_god_node as i64,
                        n.source_file,
                        n.file_type,
                        ingested_at,
                    ],
                )?;
            }
            for e in edges {
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    r#"
                    INSERT INTO code_graph_edges (
                        id, repo_root, source_id, target_id, relation,
                        confidence, confidence_score, ingested_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                    rusqlite::params![
                        id,
                        repo_root,
                        e.source_id,
                        e.target_id,
                        e.relation,
                        e.confidence,
                        e.confidence_score,
                        ingested_at,
                    ],
                )?;
            }
            Ok(())
        })
    }

    pub fn count_code_graph_nodes(&self, repo_root: &Path) -> Result<usize> {
        let repo_str = repo_root.display().to_string();
        self.with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM code_graph_nodes WHERE repo_root = ?1",
                [repo_str],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    pub fn list_god_nodes(&self, repo_root: &Path, limit: usize) -> Result<Vec<String>> {
        let repo_str = repo_root.display().to_string();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT label FROM code_graph_nodes WHERE repo_root = ?1 AND is_god_node = 1 LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![repo_str, limit as i64], |r| {
                r.get::<_, String>(0)
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    }

    pub fn search_code_graph_labels(
        &self,
        repo_root: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<super::types::CodeContextNode>> {
        let repo_str = repo_root.display().to_string();
        let pattern = format!("%{}%", query.split_whitespace().next().unwrap_or(query));
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT n.label, e.relation, t.label
                FROM code_graph_nodes n
                LEFT JOIN code_graph_edges e ON e.repo_root = n.repo_root AND e.source_id = n.graphify_id
                LEFT JOIN code_graph_nodes t ON t.repo_root = n.repo_root AND t.graphify_id = e.target_id
                WHERE n.repo_root = ?1 AND n.label LIKE ?2
                LIMIT ?3
                "#,
            )?;
            let rows = stmt.query_map(rusqlite::params![repo_str, pattern, limit as i64], |r| {
                Ok(super::types::CodeContextNode {
                    label: r.get(0)?,
                    relation: r.get(1)?,
                    target: r.get(2)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    }

    pub fn last_code_graph_ingest(&self, repo_root: &Path) -> Result<Option<i64>> {
        let repo_str = repo_root.display().to_string();
        self.with_conn(|conn| {
            let v: Option<i64> = conn
                .query_row(
                    "SELECT MAX(ingested_at) FROM code_graph_nodes WHERE repo_root = ?1",
                    [repo_str],
                    |r| r.get(0),
                )
                .ok();
            Ok(v)
        })
    }

    // ── KG Phase 2: symbol navigation queries ────────────────

    /// Search symbols in code_graph_nodes using SQL LIKE on label.
    pub fn search_symbols(
        &self,
        repo_root: &str,
        query: &str,
        language: Option<&str>,
        file: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SymbolMatch>> {
        self.with_conn(|conn| {
            let pattern = format!("%{}%", query);
            let mut sql = String::from(
                "SELECT label, source_file, file_type, start_line, end_line, ast_symbol
                 FROM code_graph_nodes
                 WHERE repo_root = ?1 AND (label LIKE ?2 OR ast_symbol LIKE ?2)"
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
                Box::new(repo_root.to_string()),
                Box::new(pattern),
            ];
            if let Some(lang) = language {
                let idx = params.len() + 1;
                sql.push_str(&format!(" AND file_type = ?{idx}"));
                params.push(Box::new(format!("ast:{lang}")));
            }
            if let Some(f) = file {
                let idx = params.len() + 1;
                sql.push_str(&format!(" AND source_file LIKE ?{idx}"));
                params.push(Box::new(format!("%{f}%")));
            }
            sql.push_str(" ORDER BY start_line LIMIT ?");
            let limit_idx = params.len() + 1;
            params.push(Box::new(limit as i64));
            sql.push_str(&limit_idx.to_string());

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |r| {
                Ok(SymbolMatch {
                    label: r.get(0)?,
                    source_file: r.get(1)?,
                    file_type: r.get(2)?,
                    start_line: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as usize,
                    end_line: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as usize,
                    ast_symbol: r.get::<_, Option<String>>(5)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    }

    /// Get a single symbol definition by name (and optionally file).
    pub fn get_symbol_definition(
        &self,
        repo_root: &str,
        name: &str,
        file: Option<&str>,
    ) -> Result<Option<SymbolMatch>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT label, source_file, file_type, start_line, end_line, ast_symbol
                 FROM code_graph_nodes
                 WHERE repo_root = ?1 AND (label LIKE ?2 OR label = ?2)"
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
                Box::new(repo_root.to_string()),
                Box::new(format!("%.{name}")),
            ];
            if let Some(f) = file {
                let idx = params.len() + 1;
                sql.push_str(&format!(" AND source_file LIKE ?{idx}"));
                params.push(Box::new(format!("%{f}%")));
            }
            sql.push_str(" LIMIT 1");

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let row = stmt.query_row(param_refs.as_slice(), |r| {
                Ok(SymbolMatch {
                    label: r.get(0)?,
                    source_file: r.get(1)?,
                    file_type: r.get(2)?,
                    start_line: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as usize,
                    end_line: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as usize,
                    ast_symbol: r.get::<_, Option<String>>(5)?,
                })
            });
            Ok(row.ok())
        })
    }

    /// Get all symbols in a file, sorted by line.
    pub fn get_file_outline(
        &self,
        repo_root: &str,
        file_path: &str,
    ) -> Result<Vec<SymbolMatch>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT label, source_file, file_type, start_line, end_line, ast_symbol
                 FROM code_graph_nodes
                 WHERE repo_root = ?1 AND source_file = ?2 AND file_type LIKE 'ast:%'
                 ORDER BY start_line"
            )?;
            let rows = stmt.query_map(rusqlite::params![repo_root, file_path], |r| {
                Ok(SymbolMatch {
                    label: r.get(0)?,
                    source_file: r.get(1)?,
                    file_type: r.get(2)?,
                    start_line: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as usize,
                    end_line: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as usize,
                    ast_symbol: r.get::<_, Option<String>>(5)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    }

    /// Find callers of a symbol via code_graph_edges with relation = 'calls'.
    pub fn find_callers(
        &self,
        repo_root: &str,
        symbol_name: &str,
    ) -> Result<Vec<SymbolMatch>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT n.label, n.source_file, n.file_type, n.start_line, n.end_line, n.ast_symbol
                FROM code_graph_edges e
                JOIN code_graph_nodes n ON n.repo_root = e.repo_root AND n.graphify_id = e.source_id
                WHERE e.repo_root = ?1
                  AND e.relation = 'calls'
                  AND e.target_id IN (
                    SELECT graphify_id FROM code_graph_nodes
                    WHERE repo_root = ?1 AND (label LIKE ?2 OR label = ?3)
                  )
                LIMIT 50
                "#,
            )?;
            let pattern = format!("%.{symbol_name}");
            let rows = stmt.query_map(rusqlite::params![repo_root, pattern, pattern], |r| {
                Ok(SymbolMatch {
                    label: r.get(0)?,
                    source_file: r.get(1)?,
                    file_type: r.get(2)?,
                    start_line: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as usize,
                    end_line: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as usize,
                    ast_symbol: r.get::<_, Option<String>>(5)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    }

    /// Recursive BFS over edges starting from a symbol, up to `depth` hops.
    pub fn get_local_graph(
        &self,
        repo_root: &str,
        symbol_name: &str,
        depth: usize,
    ) -> Result<Vec<LocalGraphEdge>> {
        use std::collections::{HashSet, VecDeque};
        let max_depth = depth.min(5);
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut edges = Vec::new();

        // Seed: find the graphify_id for this symbol
        let seed_ids = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT graphify_id FROM code_graph_nodes WHERE repo_root = ?1 AND (label LIKE ?2 OR label = ?3) LIMIT 10"
            )?;
            let pattern = format!("%.{symbol_name}");
            let rows = stmt.query_map(rusqlite::params![repo_root, pattern, pattern], |r| {
                r.get::<_, String>(0)
            })?;
            Ok::<Vec<String>, anyhow::Error>(rows.filter_map(|r| r.ok()).collect())
        })?;

        for sid in &seed_ids {
            visited.insert(sid.clone());
            queue.push_back((sid.clone(), 0));
        }

        while let Some((current, d)) = queue.pop_front() {
            if d >= max_depth {
                continue;
            }
            // Outgoing edges
            let outgoing = self.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT e.source_id, e.target_id, e.relation,
                           sn.label AS source_label, tn.label AS target_label
                    FROM code_graph_edges e
                    LEFT JOIN code_graph_nodes sn ON sn.repo_root = e.repo_root AND sn.graphify_id = e.source_id
                    LEFT JOIN code_graph_nodes tn ON tn.repo_root = e.repo_root AND tn.graphify_id = e.target_id
                    WHERE e.repo_root = ?1 AND e.source_id = ?2
                    LIMIT 20
                    "#
                )?;
                let rows = stmt.query_map(rusqlite::params![repo_root, current], |r| {
                    Ok(LocalGraphEdge {
                        source_id: r.get(0)?,
                        target_id: r.get(1)?,
                        relation: r.get(2)?,
                        source_label: r.get(3)?,
                        target_label: r.get(4)?,
                    })
                })?;
                Ok::<Vec<LocalGraphEdge>, anyhow::Error>(rows.filter_map(|r| r.ok()).collect())
            })?;
            for e in &outgoing {
                if visited.insert(e.target_id.clone()) {
                    queue.push_back((e.target_id.clone(), d + 1));
                }
            }
            edges.extend(outgoing);
        }
        Ok(edges)
    }
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

impl BrainStore {
    /// Upsert AST-derived edges into code_graph_edges.
    /// Uses INSERT OR IGNORE to avoid duplicates on (repo_root, source_id, target_id, relation).
    pub fn upsert_ast_edges(
        &self,
        repo_root: &str,
        edges: &[AstEdgeRow],
        ingested_at: i64,
    ) -> Result<()> {
        self.with_conn(|conn| {
            for e in edges {
                let id = format!("{repo_root}:{}:{}:{}", e.source_id, e.target_id, e.relation);
                conn.execute(
                    "INSERT OR IGNORE INTO code_graph_edges (id, repo_root, source_id, target_id, relation, confidence, ingested_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'ast', ?6)",
                    rusqlite::params![id, repo_root, e.source_id, e.target_id, e.relation, ingested_at],
                )?;
            }
            Ok(())
        })
    }

    /// Delete all AST-derived edges for a repo (relation = 'ast').
    pub fn delete_ast_edges(&self, repo_root: &str) -> Result<usize> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM code_graph_edges WHERE repo_root = ?1 AND confidence = 'ast'",
                [repo_root],
            )?;
            Ok(n)
        })
    }
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

#[derive(Debug, Clone, Serialize)]
pub struct LocalGraphEdge {
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub source_label: Option<String>,
    pub target_label: Option<String>,
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
