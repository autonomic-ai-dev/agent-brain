use anyhow::{bail, Context, Result};
use serde::Deserialize;

const EMBEDDED_REGISTRY: &str = include_str!("../../registry/packages.json");

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

pub fn load_curated_registry() -> Result<CuratedRegistryFile> {
    serde_json::from_str(EMBEDDED_REGISTRY).context("parse embedded package registry")
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
        let entry = reg
            .aliases
            .get(alias)
            .with_context(|| format!("unknown package alias '@{alias}'. Run: agent-brain registry list"))?;
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
    fn embedded_registry_has_nextjs() {
        let aliases = list_aliases().unwrap();
        assert!(aliases.iter().any(|a| a.alias == "nextjs"));
    }

    #[test]
    fn resolves_supervisor_bundle_alias() {
        let resolved = lookup_alias("@supervisor").unwrap().unwrap();
        assert_eq!(resolved, ResolvedAlias::Bundle("supervisor".into()));
    }
}
