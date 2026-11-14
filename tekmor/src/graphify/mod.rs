use std::path::Path;

use crate::code_nav::CodeContext;
use crate::db::store::BrainStore;

mod rows;

pub use crate::code_nav::{ScratchpadEntry, SymbolMatch};
pub use rows::{AstCodeNodeRow, AstEdgeRow, CodeGraphEdgeRow, CodeGraphNodeRow, LocalGraphEdge};

pub fn route_code_context(
    store: &BrainStore,
    repo_root: &Path,
    user_message: &str,
    max_tokens: usize,
) -> Option<CodeContext> {
    if store.count_code_graph_nodes(repo_root).ok()? == 0 {
        return None;
    }
    let gods = store.list_god_nodes(repo_root, 5).ok().unwrap_or_default();

    // SQL LIKE search on code_graph_nodes (fast, exact-match)
    let label_matches = store
        .search_code_graph_labels(repo_root, user_message, 5)
        .ok()
        .unwrap_or_default();

    // BM25 semantic search on indexed_items (scoped to repo)
    let repo_str = repo_root.display().to_string();
    let semantic_matches = store
        .search_code_context_hybrid(&repo_str, user_message, 5)
        .ok()
        .unwrap_or_default();

    // Merge: SQL LIKE results first, then BM25 results (deduped by label)
    let mut seen = std::collections::HashSet::new();
    let mut relevant_nodes = Vec::new();
    for n in label_matches.iter().chain(semantic_matches.iter()) {
        if seen.insert(&n.label) {
            relevant_nodes.push(n.clone());
        }
    }

    let last_ingested = store.last_code_graph_ingest(repo_root).ok().flatten();
    let graph_path = repo_root.join("graphify-out").join("graph.json");
    let graph_mtime = graph_path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let graph_stale = match (graph_mtime, last_ingested) {
        (Some(gm), Some(li)) => gm > li,
        (Some(_), None) => true,
        _ => false,
    };
    let ctx = CodeContext {
        god_nodes: gods,
        relevant_nodes,
        graph_stale,
        last_ingested_at: last_ingested,
    };
    let _ = max_tokens;
    Some(ctx)
}

pub fn rpc(home: &Path, tool: &str, args: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    use anyhow::{bail, Context};
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let bin = crate::extensions::bin_dir(home).join("tekmor-graphify");
    let mut child = Command::new(&bin)
        .arg("rpc")
        .arg(tool)
        .arg(args.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| {
            format!(
                "graphify extension is not installed ({}); install it with ext add graphify",
                bin.display()
            )
        })?;
    let stdout = child.stdout.take().context("graphify extension stdout")?;
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line)?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    if line.trim().is_empty() {
        bail!("graphify extension returned no output for {tool}");
    }
    Ok(serde_json::from_str(line.trim())?)
}
