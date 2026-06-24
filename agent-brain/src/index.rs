use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::config::Config;
use crate::db::store::{content_hash, BrainStore, IndexBatchItem};
use crate::embed::Embedder;
use crate::graphify::{AstCodeNodeRow, AstEdgeRow};
use crate::types::ItemType;

/// Maximum batch size for ONNX embedding calls.
/// fastembed's internal batching handles larger sizes efficiently;
/// this prevents unbounded memory from an enormous single batch.
const EMBED_BATCH_SIZE: usize = 64;

pub fn sync_index(
    store: &BrainStore,
    config: &Config,
    embedder: &Embedder,
    cwd: Option<&Path>,
) -> Result<usize> {
    sync_index_opts(store, config, embedder, cwd, false, true)
}

pub fn sync_index_opts(
    store: &BrainStore,
    config: &Config,
    embedder: &Embedder,
    cwd: Option<&Path>,
    changed_only: bool,
    enable_ast_index: bool,
) -> Result<usize> {
    let roots = config.default_index_roots(cwd);
    let mut batch: Vec<UnembeddedItem> = Vec::new();

    // Pre-load known mtimes for mtime-based skip
    let known_mtimes = if changed_only {
        store.get_all_indexed_mtimes()?
    } else {
        std::collections::HashMap::new()
    };

    // Phase 1: walk files, parse, hash-check, collect changed items
    let mut file_tasks = Vec::new();
    for root in &roots {
        if root.is_file() {
            file_tasks.push((root.to_path_buf(), None, package_context(root, &config.home)));
            continue;
        }
        if !root.exists() {
            continue;
        }
        let pkg_ctx = package_context(root, &config.home);
        for entry in WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if should_skip(path) {
                continue;
            }
            // Mtime skip: if file mtime matches stored mtime, skip entirely
            if changed_only {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if let Ok(modified) = metadata.modified() {
                        let mtime = modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        if known_mtimes.get(path.to_str().unwrap_or("")) == Some(&mtime) {
                            continue;
                        }
                    }
                }
            }
            let repo = cwd.and_then(crate::config::find_repo_root);
            file_tasks.push((path.to_path_buf(), repo, pkg_ctx.clone()));
        }
    }

    // Compute mtimes for new items before the parallel section
    let file_mtime_of = |path: &Path| -> Option<i64> {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
    };

    use rayon::prelude::*;
    let ast_nodes = Mutex::new(Vec::new());
    let ast_edges = Mutex::new(Vec::new());
    let parsed_items: Vec<UnembeddedItem> = file_tasks
        .into_par_iter()
        .flat_map(|(path, repo, pkg_ctx)| {
            let mut items = Vec::new();
            let mtime = file_mtime_of(&path);
            if let Some(item) = parse_file(&path, repo.as_deref(), pkg_ctx) {
                let hash = content_hash(&item.text);
                items.push(UnembeddedItem { item, hash, mtime });
            }
            if enable_ast_index {
                if let Ok((ast_symbols, ast_extracted_edges)) =
                    crate::ast_indexer::index_file(&path)
                {
                    for symbol in &ast_symbols {
                        let text = format!(
                            "{} {} {} {}",
                            symbol.symbol_name, symbol.symbol_kind, symbol.language, symbol.content
                        );
                        let hash = content_hash(&text);
                        let source_path = symbol.file_path.clone();
                        let item = ParsedItem {
                            item_type: ItemType::Skill,
                            topic: format!("{}/{}", symbol.language, symbol.symbol_name),
                            text: text.chars().take(800).collect(),
                            source_path,
                            scope: "project".into(),
                            scope_key: path.to_str().map(|s| s.to_string()),
                        };
                        items.push(UnembeddedItem { item, hash, mtime });

                        if let Some(ref repo_root) = repo {
                            if let Ok(mut guard) = ast_nodes.lock() {
                                guard.push(AstCodeNodeRow {
                                    repo_root: repo_root.display().to_string(),
                                    symbol_name: symbol.symbol_name.clone(),
                                    symbol_kind: symbol.symbol_kind.clone(),
                                    content: symbol.content.clone(),
                                    source_file: symbol.file_path.clone(),
                                    start_line: symbol.start_line,
                                    end_line: symbol.end_line,
                                    language: symbol.language.clone(),
                                    doc_comment: symbol.doc_comment.clone(),
                                });
                            }
                        }
                    }
                    if let Some(ref repo_root) = repo {
                        if let Ok(mut guard) = ast_edges.lock() {
                            for edge in &ast_extracted_edges {
                                guard.push(AstEdgeRow {
                                    repo_root: repo_root.display().to_string(),
                                    source_id: edge.source_symbol.clone(),
                                    target_id: edge.target_symbol.clone(),
                                    relation: edge.relation.clone(),
                                    source_file: edge.source_file.clone(),
                                    start_line: edge.start_line,
                                });
                            }
                        }
                    }
                }
            }
            items
        })
        .collect();

    for unemb in parsed_items {
        if store
            .indexed_item_current_hash(&unemb.item.source_path)?
            .as_deref()
            != Some(unemb.hash.as_str())
        {
            batch.push(unemb);
        } else if changed_only {
            // Update mtime even for unchanged items so we know they were checked
            if let Some(mtime) = unemb.mtime {
                store.set_indexed_item_mtime(&unemb.item.source_path, mtime)?;
            }
        }
    }

    // Index workflow definitions
    let workflow_dirs = &config.workflow_dirs;
    let workflows = crate::workflow_indexer::discover_workflows(workflow_dirs);
    for (path, wf) in &workflows {
        let source_path = path.display().to_string();
        let hash = content_hash(&serde_json::to_string(wf).unwrap_or_default());
        if store.indexed_item_current_hash(&source_path)?.as_deref() == Some(hash.as_str()) {
            if changed_only {
                if let Ok(m) = std::fs::metadata(path) {
                    if let Ok(t) = m.modified() {
                        if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                            store.set_indexed_item_mtime(&source_path, d.as_secs() as i64)?;
                        }
                    }
                }
            }
            continue;
        }
        let mtime = file_mtime_of(path);
        let text = serde_json::to_string_pretty(wf).unwrap_or_default();
        let item = ParsedItem {
            item_type: ItemType::Workflow,
            topic: wf.name.clone(),
            text,
            source_path,
            scope: "global".into(),
            scope_key: None,
        };
        batch.push(UnembeddedItem { item, hash, mtime });
    }

    if batch.is_empty() {
        return Ok(0);
    }

    // Phase 2: batch-embed all collected items
    // Process in sub-batches of EMBED_BATCH_SIZE to keep memory bounded
    let mut db_items: Vec<IndexBatchItem> = Vec::with_capacity(batch.len());
    for chunk in batch.chunks(EMBED_BATCH_SIZE) {
        let texts: Vec<String> = chunk
            .iter()
            .map(|ub| format!("{} {}", ub.item.topic, ub.item.text))
            .collect();
        let embeddings = embedder.embed_batch(&texts)?;
        for (ub, embedding) in chunk.iter().zip(embeddings.into_iter()) {
            db_items.push(IndexBatchItem {
                id: Uuid::new_v4().to_string(),
                item_type: ub.item.item_type,
                topic: ub.item.topic.clone(),
                text: ub.item.text.clone(),
                source_path: ub.item.source_path.clone(),
                scope: ub.item.scope.clone(),
                scope_key: ub.item.scope_key.clone(),
                content_hash: ub.hash.clone(),
                embedding,
                file_mtime: ub.mtime,
            });
        }
    }

    // Phase 3: batch-upsert in a single SQLite transaction
    store.upsert_indexed_items_batch(&db_items)?;
    store.bump_index_version()?;

    // Phase 4: build complete code graph from AST data (no external graphify binary needed)
    let ast_node_list = ast_nodes.into_inner().unwrap_or_default();
    let ast_edge_list = ast_edges.into_inner().unwrap_or_default();
    if !ast_node_list.is_empty() || !ast_edge_list.is_empty() {
        let mut by_repo: HashMap<String, (Vec<AstCodeNodeRow>, Vec<AstEdgeRow>)> = HashMap::new();
        for node in ast_node_list {
            by_repo
                .entry(node.repo_root.clone())
                .or_default()
                .0
                .push(node);
        }
        for edge in ast_edge_list {
            by_repo
                .entry(edge.repo_root.clone())
                .or_default()
                .1
                .push(edge);
        }
        let now = chrono::Utc::now().timestamp();
        for (repo_root, (nodes, edges)) in &by_repo {
            store.build_code_graph_from_ast(repo_root, nodes, edges, now)?;
        }
    }

    Ok(db_items.len())
}

/// Internal helper pairing a parsed item with its content hash.
struct UnembeddedItem {
    item: ParsedItem,
    hash: String,
    mtime: Option<i64>,
}

fn should_skip(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/node_modules/")
        || s.contains("/target/")
        || s.contains("/.git/")
        || s.contains("/graphify-out/")
        || s.contains("/venv/")
        || s.contains("/.venv/")
        || s.contains("/opt/")
        || s.contains("/.autonomic/")
        || s.contains("/.pytest_cache/")
        || s.contains("/dist/")
        || s.contains("/build/")
        || s.contains("/__pycache__/")
        || s.contains("/.next/")
        || s.contains("/.cache/")
        || s.contains("/vendor/")
        || s.ends_with(".pyc")
        || s.ends_with(".pyo")
        || s.ends_with(".so")
        || s.ends_with(".dylib")
        || s.ends_with(".dll")
        || s.ends_with(".exe")
        || s.ends_with(".bin")
}

struct ParsedItem {
    item_type: ItemType,
    topic: String,
    text: String,
    source_path: String,
    scope: String,
    scope_key: Option<String>,
}

fn package_context(path: &Path, home: &Path) -> Option<String> {
    let packages = home.join("packages");
    path.strip_prefix(&packages)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|c| c.as_os_str().to_string_lossy().to_string())
}

fn parse_file(path: &Path, repo: Option<&Path>, package: Option<String>) -> Option<ParsedItem> {
    let content = fs::read_to_string(path).ok()?;
    let source_path = path.display().to_string();
    let file_name = path.file_name()?.to_string_lossy().to_string();

    let (item_type, topic, text) = if path.ends_with("SKILL.md") {
        let name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| file_name.clone());
        (
            ItemType::Skill,
            name.clone(),
            extract_skill_text(&content, &name),
        )
    } else if path
        .parent()
        .map(|p| p.ends_with("commands"))
        .unwrap_or(false)
        || path.components().any(|c| c.as_os_str() == "commands")
    {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(file_name.clone());
        (
            ItemType::Skill,
            format!("command:{name}"),
            extract_agent_text(&content, &name),
        )
    } else if path
        .parent()
        .map(|p| p.ends_with("agents"))
        .unwrap_or(false)
        || path.components().any(|c| c.as_os_str() == "agents")
    {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(file_name.clone());
        (
            ItemType::Agent,
            name.clone(),
            extract_agent_text(&content, &name),
        )
    } else if file_name.ends_with(".mdc")
        || file_name == "CLAUDE.md"
        || file_name == "AGENTS.md"
        || file_name == ".cursorrules"
        || file_name.ends_with(".md")
    {
        (
            ItemType::Rule,
            file_name.clone(),
            content.chars().take(2000).collect(),
        )
    } else {
        return None;
    };

    let (scope, scope_key) = if let Some(package) = package {
        ("package".into(), Some(package))
    } else if let Some(repo) = repo {
        ("project".into(), Some(repo.display().to_string()))
    } else {
        ("global".into(), None)
    };

    Some(ParsedItem {
        item_type,
        topic,
        text,
        source_path,
        scope,
        scope_key,
    })
}

/// Build indexed text for a SKILL.md body (used by filesystem index and skills.sh import).
pub fn skill_index_text(content: &str, name: &str) -> String {
    extract_skill_text(content, name)
}

fn extract_skill_text(content: &str, name: &str) -> String {
    let mut parts = vec![name.to_string()];
    let body = if let Some((front, body)) = split_yaml_frontmatter(content) {
        if let Some(desc) = yaml_scalar_field(front, "description") {
            parts.push(desc);
        }
        if let Some(yaml_name) = yaml_scalar_field(front, "name") {
            if yaml_name != name {
                parts.push(yaml_name);
            }
        }
        body.to_string()
    } else {
        content.to_string()
    };
    if let Some(activation) = extract_activation_section(&body) {
        parts.push(activation);
    }
    let body_snippet: String = body.lines().take(20).collect::<Vec<_>>().join(" ");
    if !body_snippet.is_empty() {
        parts.push(body_snippet);
    }
    parts.join(" ").chars().take(800).collect()
}

/// Pull "When to use / activate" bullets from skill body — primary routing signal.
fn extract_activation_section(body: &str) -> Option<String> {
    let lower = body.to_lowercase();
    let markers = [
        "when to activate",
        "when to use",
        "use when",
        "triggers",
        "trigger when",
    ];
    let start = markers.iter().filter_map(|m| lower.find(m)).min()?;
    let slice = &body[start..];
    let section: String = slice
        .lines()
        .take(12)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if section.len() < 20 {
        return None;
    }
    Some(section.chars().take(400).collect())
}

fn split_yaml_frontmatter(content: &str) -> Option<(&str, &str)> {
    let rest = content.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let front = &rest[..end];
    let body = rest[end + 4..].trim_start();
    Some((front, body))
}

fn yaml_scalar_field(frontmatter: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) {
            let mut val = trimmed[prefix.len()..].trim().to_string();
            if val.starts_with('>') || val.starts_with('|') {
                continue;
            }
            val = val.trim_matches('"').trim_matches('\'').to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

fn extract_agent_text(content: &str, name: &str) -> String {
    let summary = content.lines().take(15).collect::<Vec<_>>().join(" ");
    format!("{name} {summary}").chars().take(800).collect()
}
