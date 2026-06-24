//! v0.30 — Knowledge Graph Phase 1-4: AST symbol storage, edge extraction, in-process graph building.

use std::path::Path;

use agent_brain::db::store::BrainStore;
use agent_brain::graphify::{AstCodeNodeRow, AstEdgeRow, CodeGraphNodeRow};
use tempfile::TempDir;

fn open_store(dir: &TempDir) -> BrainStore {
    let db_path = dir.path().join("brain.db");
    BrainStore::open(&db_path).unwrap()
}

fn ast_nodes_for_root(repo_root: &str) -> Vec<AstCodeNodeRow> {
    vec![
        AstCodeNodeRow {
            repo_root: repo_root.to_string(),
            symbol_name: "handle_request".into(),
            symbol_kind: "function".into(),
            content: "fn handle_request() { Ok(()) }".into(),
            source_file: "src/handler.rs".into(),
            start_line: 10,
            end_line: 12,
            language: "rust".into(),
            doc_comment: Some("Handles the incoming request.".into()),
        },
        AstCodeNodeRow {
            repo_root: repo_root.to_string(),
            symbol_name: "AppConfig".into(),
            symbol_kind: "struct".into(),
            content: "struct AppConfig { port: u16 }".into(),
            source_file: "src/config.rs".into(),
            start_line: 1,
            end_line: 3,
            language: "rust".into(),
            doc_comment: None,
        },
    ]
}

fn count_ast_nodes(store: &BrainStore, repo_root: &str) -> usize {
    store
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT COUNT(*) FROM code_graph_nodes WHERE repo_root = ?1 AND file_type LIKE 'ast:%'",
                )
                .unwrap();
            let n: i64 = stmt.query_row(rusqlite::params![repo_root], |r| r.get(0)).unwrap();
            Ok(n as usize)
        })
        .unwrap()
}

fn count_code_graph_nodes(store: &BrainStore, repo_root: &str) -> usize {
    store
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT COUNT(*) FROM code_graph_nodes WHERE repo_root = ?1")
                .unwrap();
            let n: i64 = stmt.query_row(rusqlite::params![repo_root], |r| r.get(0)).unwrap();
            Ok(n as usize)
        })
        .unwrap()
}

fn count_code_graph_edges(store: &BrainStore, repo_root: &str) -> usize {
    store
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT COUNT(*) FROM code_graph_edges WHERE repo_root = ?1")
                .unwrap();
            let n: i64 = stmt.query_row(rusqlite::params![repo_root], |r| r.get(0)).unwrap();
            Ok(n as usize)
        })
        .unwrap()
}

#[test]
fn migration_v15_adds_columns() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);

    store
        .with_conn(|conn| {
            let mut stmt = conn.prepare("PRAGMA table_info(code_graph_nodes)").unwrap();
            let cols: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            assert!(cols.contains(&"ast_symbol".into()), "ast_symbol column missing");
            assert!(cols.contains(&"embedding_id".into()), "embedding_id column missing");
            assert!(cols.contains(&"start_line".into()), "start_line column missing");
            assert!(cols.contains(&"end_line".into()), "end_line column missing");
            Ok(())
        })
        .unwrap();
}

#[test]
fn upsert_ast_code_nodes_inserts_new_nodes() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);
    let repo = "/tmp/test-repo";

    let nodes = ast_nodes_for_root(repo);
    store.upsert_ast_code_nodes(repo, &nodes, 1000).unwrap();

    assert_eq!(count_ast_nodes(&store, repo), 2);
    assert_eq!(count_code_graph_nodes(&store, repo), 2);
}

#[test]
fn upsert_ast_code_nodes_updates_existing_by_graphify_id() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);
    let repo = "/tmp/test-repo";

    let nodes = ast_nodes_for_root(repo);
    store.upsert_ast_code_nodes(repo, &nodes, 1000).unwrap();

    let updated = vec![AstCodeNodeRow {
        repo_root: repo.to_string(),
        symbol_name: "handle_request".into(),
        symbol_kind: "function".into(),
        content: "fn handle_request() -> Result<()> { Ok(()) }".into(),
        source_file: "src/handler.rs".into(),
        start_line: 10,
        end_line: 14,
        language: "Rust".into(),
            doc_comment: Some("Updated doc.".into()),
        }];
    store.upsert_ast_code_nodes(repo, &updated, 2000).unwrap();

    // handle_request updated in place, AppConfig unchanged
    assert_eq!(count_code_graph_nodes(&store, repo), 2);
    store
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT start_line, end_line, ast_symbol FROM code_graph_nodes WHERE graphify_id = ?1")
                .unwrap();
            let graphify_id = "ast:src/handler.rs:handle_request";
            let (start, end, ast_json): (i64, i64, String) = stmt
                .query_row(rusqlite::params![graphify_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .unwrap();
            assert_eq!(start, 10);
            assert_eq!(end, 14);
            assert!(ast_json.contains("Updated doc."), "ast_symbol should contain updated doc");
            Ok(())
        })
        .unwrap();
}

#[test]
fn ast_nodes_coexist_with_graphify_nodes() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);
    let repo = "/tmp/test-repo";

    let graphify_nodes = vec![CodeGraphNodeRow {
        graphify_id: "module:main".into(),
        label: "main_module".into(),
        community_id: Some(1),
        is_god_node: false,
        source_file: Some("src/main.rs".into()),
        file_type: Some("rust".into()),
    }];
    store
        .replace_code_graph(repo, &graphify_nodes, &[], 100)
        .unwrap();

    let ast_nodes = ast_nodes_for_root(repo);
    store.upsert_ast_code_nodes(repo, &ast_nodes, 100).unwrap();

    assert_eq!(count_code_graph_nodes(&store, repo), 3);
    assert_eq!(count_ast_nodes(&store, repo), 2);
}

#[test]
fn replace_code_graph_clears_ast_nodes() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);
    let repo = "/tmp/test-repo";

    let ast_nodes = ast_nodes_for_root(repo);
    store.upsert_ast_code_nodes(repo, &ast_nodes, 100).unwrap();
    assert_eq!(count_code_graph_nodes(&store, repo), 2);

    store
        .replace_code_graph(repo, &[], &[], 200)
        .unwrap();
    assert_eq!(count_code_graph_nodes(&store, repo), 0);
}

#[test]
fn count_code_graph_nodes_includes_ast_nodes() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);
    let repo = "/tmp/test-repo";

    assert_eq!(store.count_code_graph_nodes(Path::new(repo)).unwrap(), 0);

    let nodes = ast_nodes_for_root(repo);
    store.upsert_ast_code_nodes(repo, &nodes, 100).unwrap();

    assert_eq!(store.count_code_graph_nodes(Path::new(repo)).unwrap(), 2);
}

#[test]
fn build_code_graph_from_ast_creates_queryable_nodes_and_edges() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir);
    let repo = "/tmp/test-repo";

    let nodes = vec![
        AstCodeNodeRow {
            repo_root: repo.to_string(),
            symbol_name: "main".into(),
            symbol_kind: "function".into(),
            content: "fn main() { helper() }".into(),
            source_file: "src/main.rs".into(),
            start_line: 1,
            end_line: 3,
            language: "rust".into(),
            doc_comment: None,
        },
        AstCodeNodeRow {
            repo_root: repo.to_string(),
            symbol_name: "helper".into(),
            symbol_kind: "function".into(),
            content: "fn helper() {}".into(),
            source_file: "src/helper.rs".into(),
            start_line: 5,
            end_line: 7,
            language: "rust".into(),
            doc_comment: None,
        },
    ];

    let edges = vec![
        AstEdgeRow {
            repo_root: repo.to_string(),
            source_id: "main".into(),
            target_id: "helper".into(),
            relation: "calls".into(),
            source_file: "src/main.rs".into(),
            start_line: 2,
        },
    ];

    store
        .build_code_graph_from_ast(repo, &nodes, &edges, 1000)
        .unwrap();

    assert_eq!(count_code_graph_nodes(&store, repo), 2);
    assert_eq!(count_code_graph_edges(&store, repo), 1);

    // Verify graphify_id format: "{language}.{symbol_name}"
    store.with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT graphify_id, source_file, file_type FROM code_graph_nodes WHERE repo_root = ?1 ORDER BY graphify_id")
            .unwrap();
        let rows: Vec<(String, String, String)> = stmt
            .query_map(rusqlite::params![repo], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "rust.helper");
        assert!(rows[0].1.ends_with("helper.rs"));
        assert_eq!(rows[0].2, "ast:rust");
        assert_eq!(rows[1].0, "rust.main");
        assert!(rows[1].1.ends_with("main.rs"));
        Ok(())
    }).unwrap();

    // Verify edges reference graphify_id format via JOIN
    store.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT e.source_id, e.target_id, e.relation, n.graphify_id
                 FROM code_graph_edges e
                 JOIN code_graph_nodes n ON n.repo_root = e.repo_root AND n.graphify_id = e.target_id
                 WHERE e.repo_root = ?1"
            )
            .unwrap();
        let rows: Vec<(String, String, String, String)> = stmt
            .query_map(rusqlite::params![repo], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(rows.len(), 1, "edge should join with node by graphify_id");
        assert_eq!(rows[0].0, "rust.main", "edge source_id matches node graphify_id format");
        assert_eq!(rows[0].1, "rust.helper", "edge target_id matches node graphify_id format");
        assert_eq!(rows[0].2, "calls");
        assert_eq!(rows[0].3, "rust.helper");
        Ok(())
    }).unwrap();
}
