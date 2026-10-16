use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::registry_sync;

const EMBEDDED_REGISTRY: &str = include_str!("../../registry/packages.json");
const EMBEDDED_UTILITIES: &str = include_str!("../../registry/utilities.json");
const EMBEDDED_WORKFLOWS: &str = include_str!("../../registry/workflows.json");

#[derive(Debug, Clone, Deserialize)]
pub struct CuratedRegistryFile {
    pub version: u32,
    pub aliases: std::collections::BTreeMap<String, CuratedAlias>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CuratedAlias {
    pub description: String,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CuratedAliasInfo {
    pub alias: String,
    pub description: String,
    pub bundle: Option<String>,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UtilityInfo {
    pub name: String,
    pub alias: String,
    pub description: String,
    pub invoke: Vec<String>,
    pub when: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct UtilitiesRegistryFile {
    #[allow(dead_code)]
    version: u32,
    utilities: std::collections::BTreeMap<String, UtilityEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct UtilityEntry {
    alias: String,
    description: String,
    #[serde(default)]
    invoke: Vec<String>,
    #[serde(default)]
    when: Option<String>,
}

pub fn list_utilities() -> Result<Vec<UtilityInfo>> {
    let home = registry_home();
    let raw = registry_sync::read_registry_file(&home, "utilities.json", EMBEDDED_UTILITIES);
    let reg: UtilitiesRegistryFile =
        serde_json::from_str(&raw).context("parse utilities registry")?;
    Ok(reg
        .utilities
        .into_iter()
        .map(|(name, entry)| UtilityInfo {
            name,
            alias: entry.alias,
            description: entry.description,
            invoke: entry.invoke,
            when: entry.when,
        })
        .collect())
}

#[derive(Debug, Clone)]
pub struct WorkflowInfo {
    pub alias: String,
    pub description: String,
    pub organ: String,
    pub invoke: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkflowsRegistryFile {
    #[allow(dead_code)]
    version: u32,
    workflows: std::collections::BTreeMap<String, WorkflowEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkflowEntry {
    description: String,
    path: String,
    organ: String,
    #[serde(default)]
    invoke: String,
}

pub fn list_workflows() -> Result<Vec<WorkflowInfo>> {
    let home = registry_home();
    let raw = registry_sync::read_registry_file(&home, "workflows.json", EMBEDDED_WORKFLOWS);
    let reg: WorkflowsRegistryFile =
        serde_json::from_str(&raw).context("parse workflows registry")?;
    Ok(reg
        .workflows
        .into_iter()
        .map(|(alias, entry)| WorkflowInfo {
            alias,
            description: entry.description,
            organ: entry.organ,
            invoke: entry.invoke,
            path: entry.path,
        })
        .collect())
}

pub fn load_curated_registry() -> Result<CuratedRegistryFile> {
    let home = registry_home();
    let raw = registry_sync::read_registry_file(&home, "packages.json", EMBEDDED_REGISTRY);
    serde_json::from_str(&raw).context("parse package registry")
}

fn registry_home() -> PathBuf {
    std::env::var("AGENT_BRAIN_HOME")
        .map(PathBuf::from)
        .or_else(|_| crate::config::Config::load().map(|c| c.home))
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".agent_brain")
        })
}

pub fn list_aliases() -> Result<Vec<CuratedAliasInfo>> {
    let reg = load_curated_registry()?;
    Ok(reg
        .aliases
        .into_iter()
        .map(|(alias, entry)| CuratedAliasInfo {
            alias,
            description: entry.description,
            bundle: entry.bundle,
            packages: entry.packages,
        })
        .collect())
}

/// Resolve user input to one or more GitHub owner/repo sources.
/// Supports `@alias` (curated) and passes through normal sources unchanged.
pub fn resolve_package_inputs(input: &str) -> Result<Vec<String>> {
    if let Some(resolved) = lookup_alias(input)? {
        return Ok(resolved.into_git_sources());
    }
    Ok(vec![input.trim().to_string()])
}

/// Resolve `@alias` to bundle name or git package list.
pub fn lookup_alias(input: &str) -> Result<Option<ResolvedAlias>> {
    let trimmed = input.trim();
    if let Some(alias) = trimmed.strip_prefix('@') {
        let alias = alias.trim();
        if alias.is_empty() {
            bail!("empty package alias; use @starter, @supervisor, @nextjs, @ecc, or owner/repo");
        }
        let reg = load_curated_registry()?;
        let entry = reg.aliases.get(alias).with_context(|| {
            format!("unknown package alias '@{alias}'. Run: agent-brain registry list")
        })?;
        if let Some(bundle) = &entry.bundle {
            return Ok(Some(ResolvedAlias::Bundle(bundle.clone())));
        }
        if entry.packages.is_empty() {
            bail!("alias '@{alias}' has no packages or bundle configured");
        }
        return Ok(Some(ResolvedAlias::Git(entry.packages.clone())));
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedAlias {
    Bundle(String),
    Git(Vec<String>),
}

impl ResolvedAlias {
    pub fn into_git_sources(self) -> Vec<String> {
        match self {
            ResolvedAlias::Git(pkgs) => pkgs,
            ResolvedAlias::Bundle(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_starter_alias() {
        let sources = resolve_package_inputs("@starter").unwrap();
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().any(|s| s.contains("vercel-labs/skills")));
    }

    #[test]
    fn passes_through_owner_repo() {
        let sources = resolve_package_inputs("foo/bar").unwrap();
        assert_eq!(sources, vec!["foo/bar"]);
    }

    #[test]
    fn resolves_autonomic_core_bundle_alias() {
        let resolved = lookup_alias("@autonomic-core").unwrap().unwrap();
        assert_eq!(
            resolved,
            ResolvedAlias::Bundle("autonomic-core".into())
        );
    }

    #[test]
    fn embedded_registry_lists_workflows() {
        let workflows = list_workflows().unwrap();
        assert!(workflows.iter().any(|w| w.alias == "release-notes"));
        assert!(workflows.iter().any(|w| w.alias == "stacked-pr"));
    }

    #[test]
    fn embedded_registry_has_nextjs() {
        let aliases = list_aliases().unwrap();
        assert!(aliases.iter().any(|a| a.alias == "nextjs"));
    }

    #[test]
    fn resolves_supervisor_bundle_alias() {
        let resolved = lookup_alias("@supervisor").unwrap().unwrap();
        assert_eq!(resolved, ResolvedAlias::Bundle("supervisor".into()));
    }
    #[test]
    #[test]
    fn utilities_catalog_includes_agent_body() {
        let utils = list_utilities().unwrap();
        assert!(utils.iter().any(|u| u.name == "agent-body"));
        assert!(utils.iter().any(|u| u.name == "agent-heart"));
        assert!(utils.iter().any(|u| u.name == "agent-muscle"));
        let body = utils.iter().find(|u| u.name == "agent-body").unwrap();
        assert!(body.description.to_ascii_lowercase().contains("control plane")
            || body.description.to_ascii_lowercase().contains("prefer"));
    }

    #[test]
    fn resolves_coding_policy_bundle_alias() {
        let resolved = lookup_alias("@coding-policy").unwrap().unwrap();
        assert_eq!(
            resolved,
            ResolvedAlias::Bundle("coding-policy".into())
        );
    }

}
