use serde::{Deserialize, Serialize};
use std::path::Path;

use anyhow::Result;
use rusqlite::params;
use uuid::Uuid;

use crate::code_nav::{CodeContextNode, SymbolMatch};
use crate::db::store::BrainStore;
use crate::graphify::{
    AstCodeNodeRow, AstEdgeRow, CodeGraphEdgeRow, CodeGraphNodeRow, LocalGraphEdge,
};

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
    ) -> Result<Vec<crate::code_nav::CodeContextNode>> {
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
                Ok(crate::code_nav::CodeContextNode {
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
                 WHERE repo_root = ?1 AND (label LIKE ?2 OR ast_symbol LIKE ?2)",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(repo_root.to_string()), Box::new(pattern)];
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
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
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
                 WHERE repo_root = ?1 AND (label LIKE ?2 OR label = ?2)",
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
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
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
    pub fn get_file_outline(&self, repo_root: &str, file_path: &str) -> Result<Vec<SymbolMatch>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT label, source_file, file_type, start_line, end_line, ast_symbol
                 FROM code_graph_nodes
                 WHERE repo_root = ?1 AND source_file = ?2 AND file_type LIKE 'ast:%'
                 ORDER BY start_line",
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
    pub fn find_callers(&self, repo_root: &str, symbol_name: &str) -> Result<Vec<SymbolMatch>> {
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

    /// Delete all AST-derived edges for a repo (confidence = 'ast').
    pub fn delete_ast_edges(&self, repo_root: &str) -> Result<usize> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM code_graph_edges WHERE repo_root = ?1 AND confidence = 'ast'",
                [repo_root],
            )?;
            Ok(n)
        })
    }

    /// Phase 4: Atomically replace the AST-derived portion of the code graph.
    /// Deletes existing AST-derived nodes (ast_symbol IS NOT NULL) and edges (confidence = 'ast'),
    /// then inserts fresh nodes and edges with graphify_id = "{language}.{symbol_name}" so that
    /// queries joining e.source_id = n.graphify_id resolve correctly.
    ///
    /// Graphify-binary data (community_id, is_god_node) is left untouched.
    pub fn build_code_graph_from_ast(
        &self,
        repo_root: &str,
        nodes: &[AstCodeNodeRow],
        edges: &[AstEdgeRow],
        ingested_at: i64,
    ) -> Result<(usize, usize)> {
        fn lang_from_path(path: &str) -> Option<&'static str> {
            let ext = std::path::Path::new(path).extension()?.to_str()?;
            match ext {
                "rs" => Some("rust"),
                "ts" | "tsx" => Some("typescript"),
                "py" => Some("python"),
                "go" => Some("go"),
                _ => None,
            }
        }

        self.with_conn(|conn| {
            // Delete existing AST-derived edges + nodes
            conn.execute(
                "DELETE FROM code_graph_edges WHERE repo_root = ?1 AND confidence = 'ast'",
                [repo_root],
            )?;
            conn.execute(
                "DELETE FROM code_graph_nodes WHERE repo_root = ?1 AND ast_symbol IS NOT NULL",
                [repo_root],
            )?;

            // Insert AST-derived nodes with graphify_id = "{language}.{symbol_name}"
            for n in nodes {
                let graphify_id = format!("{}.{}", n.language, n.symbol_name);
                let id = format!("{repo_root}:{graphify_id}");
                let label = graphify_id.clone();
                let file_type = format!("ast:{}", n.language);
                let ast_json = serde_json::json!({
                    "kind": n.symbol_kind,
                    "content": n.content,
                    "doc": n.doc_comment,
                });
                conn.execute(
                    "INSERT INTO code_graph_nodes (
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
                        end_line = excluded.end_line",
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

            // Insert AST-derived edges with source_id/target_id = "{language}.{symbol_name}"
            // so they join with code_graph_nodes.graphify_id
            for e in edges {
                let lang = lang_from_path(&e.source_file).unwrap_or("unknown");
                let source_id = format!("{}.{}", lang, e.source_id);
                let target_id = format!("{}.{}", lang, e.target_id);
                let edge_id = format!("{repo_root}:{source_id}:{target_id}:{}", e.relation);
                conn.execute(
                    "INSERT OR IGNORE INTO code_graph_edges (id, repo_root, source_id, target_id, relation, confidence, ingested_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'ast', ?6)",
                    rusqlite::params![edge_id, repo_root, source_id, target_id, e.relation, ingested_at],
                )?;
            }

            Ok((nodes.len(), edges.len()))
        })
    }

    /// Phase 5: Hybrid BM25 + vector search over code graph entries in indexed_items.
    /// Returns up to `limit` relevant nodes for a codebase query, scoped to `repo_root`.
    pub fn search_code_context_hybrid(
        &self,
        repo_root: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CodeContextNode>> {
        let fts_query = crate::retrieval::fts_query_strict(query);
        let loose_query = crate::retrieval::fts_query_loose(query);

        self.with_conn(|conn| {
            let mut results: Vec<(String, f64)> = Vec::new();

            // Strict BM25 filtered by repo scope
            if fts_query != "\"\"" {
                let mut stmt = conn.prepare(
                    r#"SELECT i.topic, bm25(items_fts) AS score
                       FROM items_fts
                       JOIN indexed_items i ON i.rowid = items_fts.rowid
                       WHERE items_fts MATCH ?1
                         AND i.scope_key = ?2
                       ORDER BY score
                       LIMIT ?3"#,
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![fts_query, repo_root, (limit * 2) as i64],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
                )?;
                for row in rows.flatten() {
                    results.push(row);
                }
            }

            // Loose BM25 fallback if too few hits
            if results.len() < 3 && loose_query != fts_query && loose_query != "\"\"" {
                let mut stmt = conn.prepare(
                    r#"SELECT i.topic, bm25(items_fts) AS score
                       FROM items_fts
                       JOIN indexed_items i ON i.rowid = items_fts.rowid
                       WHERE items_fts MATCH ?1
                         AND i.scope_key = ?2
                       ORDER BY score
                       LIMIT ?3"#,
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![loose_query, repo_root, (limit * 2) as i64],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
                )?;
                for row in rows.flatten() {
                    if !results.iter().any(|(t, _)| t == &row.0) {
                        results.push(row);
                    }
                }
            }

            Ok(results
                .into_iter()
                .take(limit)
                .map(|(label, _)| CodeContextNode {
                    label,
                    relation: None,
                    target: None,
                })
                .collect())
        })
    }

    pub fn recent_symbols(&self, repo_root: &str, limit: usize) -> Result<Vec<SymbolMatch>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT label, source_file, file_type, start_line, end_line, ast_symbol
                 FROM code_graph_nodes
                 WHERE repo_root = ?1 AND ast_symbol IS NOT NULL
                 ORDER BY ingested_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![repo_root, limit as i64], |r| {
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
}

impl BrainStore {
    pub fn insert_graphify_job(
        &self,
        id: &str,
        repo_root: &str,
        trigger: &str,
        mode: &str,
        status: &str,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO graphify_jobs (
                    id, repo_root, trigger, mode, status, started_at, finished_at, error, result_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)
                "#,
                rusqlite::params![
                    id,
                    repo_root,
                    trigger,
                    mode,
                    status,
                    started_at,
                    finished_at,
                    error
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_graphify_job(
        &self,
        id: &str,
        status: &str,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        error: Option<&str>,
        result_json: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE graphify_jobs
                SET status = ?2,
                    started_at = COALESCE(?3, started_at),
                    finished_at = COALESCE(?4, finished_at),
                    error = COALESCE(?5, error),
                    result_json = COALESCE(?6, result_json)
                WHERE id = ?1
                "#,
                rusqlite::params![id, status, started_at, finished_at, error, result_json],
            )?;
            Ok(())
        })
    }

    pub fn get_graphify_job(&self, id: &str) -> Result<Option<GraphifyJobRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, repo_root, trigger, mode, status, started_at, finished_at, error, result_json FROM graphify_jobs WHERE id = ?1",
            )?;
            let mut rows = stmt.query([id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(GraphifyJobRecord {
                    id: row.get(0)?,
                    repo_root: row.get(1)?,
                    trigger: row.get(2)?,
                    mode: row.get(3)?,
                    status: row.get(4)?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    error: row.get(7)?,
                    result_json: row.get(8)?,
                }))
            } else {
                Ok(None)
            }
        })
    }
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
