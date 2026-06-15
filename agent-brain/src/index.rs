use std::fs;
use std::path::Path;

use anyhow::Result;
use walkdir::WalkDir;

use crate::config::Config;
use crate::db::store::{content_hash, BrainStore};
use crate::embed::Embedder;
use crate::types::ItemType;

pub fn sync_index(
    store: &BrainStore,
    config: &Config,
    embedder: &Embedder,
    cwd: Option<&Path>,
) -> Result<usize> {
    let roots = config.default_index_roots(cwd);
    let mut count = 0;

    for root in roots {
        if root.is_file() {
            if let Some(item) = parse_file(&root, None) {
                index_item(store, embedder, &item)?;
                count += 1;
            }
            continue;
        }
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if should_skip(path) {
                continue;
            }
            let repo = cwd.and_then(|c| crate::config::find_repo_root(c));
            if let Some(item) = parse_file(path, repo.as_deref()) {
                index_item(store, embedder, &item)?;
                count += 1;
            }
        }
    }

    store.bump_index_version()?;
    Ok(count)
}

fn should_skip(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/node_modules/")
        || s.contains("/target/")
        || s.contains("/.git/")
        || s.contains("/graphify-out/")
}

struct ParsedItem {
    item_type: ItemType,
    topic: String,
    text: String,
    source_path: String,
    scope: String,
    scope_key: Option<String>,
}

fn parse_file(path: &Path, repo: Option<&Path>) -> Option<ParsedItem> {
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
    } else if path.parent().map(|p| p.ends_with("agents")).unwrap_or(false)
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
    {
        (
            ItemType::Rule,
            file_name.clone(),
            content.chars().take(2000).collect(),
        )
    } else if file_name.ends_with(".md") {
        (
            ItemType::Rule,
            file_name.clone(),
            content.chars().take(2000).collect(),
        )
    } else {
        return None;
    };

    let (scope, scope_key) = if let Some(repo) = repo {
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

fn extract_skill_text(content: &str, name: &str) -> String {
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("---") {
            let front = &content[3..3 + end];
            return format!("{name} {front}").chars().take(800).collect();
        }
    }
    content.chars().take(800).collect()
}

fn extract_agent_text(content: &str, name: &str) -> String {
    let summary = content.lines().take(15).collect::<Vec<_>>().join(" ");
    format!("{name} {summary}").chars().take(800).collect()
}

fn index_item(store: &BrainStore, embedder: &Embedder, item: &ParsedItem) -> Result<()> {
    let hash = content_hash(&item.text);
    let embedding = embedder.embed_one(&format!("{} {}", item.topic, item.text))?;
    store.upsert_indexed_item(
        item.item_type,
        &item.topic,
        &item.text,
        &item.source_path,
        &item.scope,
        item.scope_key.as_deref(),
        &hash,
        Some(&embedding),
    )
}
