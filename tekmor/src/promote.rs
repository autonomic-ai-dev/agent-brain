//! Stage memory facts as SKILL.md drafts; human approval required before install.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::config::find_repo_root;
use crate::db::store::BrainStore;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PromoteResult {
    pub staging_id: String,
    pub skill_name: String,
    pub draft_path: PathBuf,
    pub status: String,
}

pub fn promote_fact_to_skill(
    store: &BrainStore,
    home: &Path,
    fact_id: Option<&str>,
    topic: Option<&str>,
    skill_name: Option<&str>,
) -> Result<PromoteResult> {
    let fact = resolve_fact(store, fact_id, topic)?;
    let fact_id = fact["id"].as_str().context("fact id")?;
    let topic = fact["topic"].as_str().context("fact topic")?;
    let fact_text = fact["fact"].as_str().context("fact text")?;
    let scope = fact["scope"].as_str().unwrap_or("project");
    let scope_key = fact["scope_key"].as_str();

    let skill_name = skill_name
        .map(slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slugify(topic));

    let staging_id = Uuid::new_v4().to_string();
    let staging_dir = home.join("staging").join(&staging_id);
    fs::create_dir_all(&staging_dir)?;
    let draft_path = staging_dir.join("SKILL.md");
    let target_path = resolve_skill_target(scope, scope_key, &skill_name)?;

    let body = render_skill_draft(&skill_name, topic, fact_text, fact_id);
    fs::write(&draft_path, &body)?;

    store.insert_skill_staging(
        &staging_id,
        fact_id,
        topic,
        &skill_name,
        draft_path.display().to_string().as_str(),
        target_path.as_deref(),
    )?;

    Ok(PromoteResult {
        staging_id,
        skill_name,
        draft_path,
        status: "pending".into(),
    })
}

pub fn list_staging(
    store: &BrainStore,
    status: Option<&str>,
) -> Result<Vec<crate::db::store::SkillStagingRow>> {
    store.list_skill_staging(status)
}

pub fn approve_staging(store: &BrainStore, staging_id: &str) -> Result<PathBuf> {
    let row = store
        .get_skill_staging(staging_id)?
        .with_context(|| format!("staging record not found: {staging_id}"))?;
    if row.status != "pending" {
        bail!("staging {staging_id} is already {}", row.status);
    }
    let draft = PathBuf::from(&row.draft_path);
    if !draft.exists() {
        bail!("draft missing at {}", draft.display());
    }
    let target = row
        .target_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_skill_target(&row.skill_name));
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&draft, &target)?;
    store.resolve_skill_staging(staging_id, "approved")?;
    Ok(target)
}

pub fn reject_staging(store: &BrainStore, staging_id: &str) -> Result<()> {
    let row = store
        .get_skill_staging(staging_id)?
        .with_context(|| format!("staging record not found: {staging_id}"))?;
    if row.status != "pending" {
        bail!("staging {staging_id} is already {}", row.status);
    }
    store.resolve_skill_staging(staging_id, "rejected")?;
    let draft = PathBuf::from(&row.draft_path);
    if let Some(parent) = draft.parent() {
        let _ = fs::remove_dir_all(parent);
    }
    Ok(())
}

fn resolve_fact(
    store: &BrainStore,
    fact_id: Option<&str>,
    topic: Option<&str>,
) -> Result<serde_json::Value> {
    if let Some(id) = fact_id {
        return store
            .get_fact(id)?
            .with_context(|| format!("fact not found: {id}"));
    }
    if let Some(topic) = topic {
        let scope_key = std::env::current_dir()
            .ok()
            .and_then(|c| find_repo_root(&c))
            .map(|p| p.display().to_string());
        if let Some(snap) =
            store.get_active_fact_by_topic(topic, "project", scope_key.as_deref())?
        {
            return store
                .get_fact(&snap.id)?
                .with_context(|| format!("fact not found: {}", snap.id));
        }
        if let Some(snap) = store.get_active_fact_by_topic(topic, "global", None)? {
            return store
                .get_fact(&snap.id)?
                .with_context(|| format!("fact not found: {}", snap.id));
        }
        bail!("no active fact for topic {topic}");
    }
    bail!("promote requires fact_id or topic");
}

fn resolve_skill_target(
    scope: &str,
    scope_key: Option<&str>,
    skill_name: &str,
) -> Result<Option<String>> {
    if scope == "global" {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(Some(
            home.join(".cursor")
                .join("skills")
                .join(skill_name)
                .join("SKILL.md")
                .display()
                .to_string(),
        ));
    }
    if let Some(key) = scope_key {
        return Ok(Some(
            PathBuf::from(key)
                .join(".cursor")
                .join("skills")
                .join(skill_name)
                .join("SKILL.md")
                .display()
                .to_string(),
        ));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = find_repo_root(&cwd).unwrap_or(cwd);
    Ok(Some(
        root.join(".cursor")
            .join("skills")
            .join(skill_name)
            .join("SKILL.md")
            .display()
            .to_string(),
    ))
}

fn default_skill_target(skill_name: &str) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = find_repo_root(&cwd).unwrap_or(cwd);
    root.join(".cursor")
        .join("skills")
        .join(skill_name)
        .join("SKILL.md")
}

fn render_skill_draft(skill_name: &str, topic: &str, fact: &str, fact_id: &str) -> String {
    let description = fact
        .chars()
        .take(120)
        .collect::<String>()
        .trim()
        .to_string();
    let title = topic
        .split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"---
name: {skill_name}
description: Promoted from tekmor memory ({topic}). {description}
---

# {title}

## Guidance

{fact}

## When to use

Apply when working on tasks related to **{topic}**.

## Source

Promoted from tekmor fact `{fact_id}` (pending human approval).
"#
    )
}

pub fn slugify(input: &str) -> String {
    let lower = input.trim().to_ascii_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes_topic() {
        assert_eq!(slugify("My Cool Topic!"), "my-cool-topic");
    }
}
