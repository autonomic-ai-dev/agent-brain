use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::registry_sync;

const EMBEDDED_REGISTRY: &str = include_str!("../../registry/packages.json");
const EMBEDDED_UTILITIES: &str = include_str!("../../registry/utilities.json");
const EMBEDDED_WORKFLOWS: &str = include_str!("../../registry/workflows.json");

#[derive(Debug, Clone, Deserialize)]
pub struct CuratedRegistryFile {
    #[allow(dead_code)]
    pub version: u32,
    pub aliases: std::collections::BTreeMap<String, CuratedAlias>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryJsonFile {
    #[allow(dead_code)]
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub packages: std::collections::BTreeMap<String, RegistryPackageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryPackageEntry {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub recommend_boost: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CuratedAlias {
    pub description: String,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
    /// Install automatically on `doctor --fix` / first install when missing.
    #[serde(default)]
    pub default: bool,
    /// Additive route score boost for skills/rules from this package (0 = none).
    #[serde(default)]
    pub recommend_boost: f64,
}

#[derive(Debug, Clone)]
pub struct CuratedAliasInfo {
    pub alias: String,
    pub description: String,
    pub bundle: Option<String>,
    pub packages: Vec<String>,
    pub default: bool,
    pub recommend_boost: f64,
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
    let mut reg: UtilitiesRegistryFile =
        serde_json::from_str(&raw).context("parse utilities registry")?;
    if let Ok(embedded) = serde_json::from_str::<UtilitiesRegistryFile>(EMBEDDED_UTILITIES) {
        for (name, entry) in embedded.utilities {
            reg.utilities.entry(name).or_insert(entry);
        }
    }
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
    let mut reg: WorkflowsRegistryFile =
        serde_json::from_str(&raw).context("parse workflows registry")?;
    if let Ok(embedded) = serde_json::from_str::<WorkflowsRegistryFile>(EMBEDDED_WORKFLOWS) {
        for (alias, entry) in embedded.workflows {
            reg.workflows.entry(alias).or_insert(entry);
        }
    }
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
    let reg_json_path = registry_sync::cache_dir(&home).join("registry.json");
    if reg_json_path.is_file() {
        if let Ok(raw) = std::fs::read_to_string(&reg_json_path) {
            if let Ok(parsed) = serde_json::from_str::<RegistryJsonFile>(&raw) {
                let mut aliases = std::collections::BTreeMap::new();
                for (name, entry) in &parsed.packages {
                    let (bundle, packages) = if let Some(p) = &entry.path {
                        let bundle_name = p.strip_prefix("packages/").unwrap_or(p).to_string();
                        (Some(bundle_name), Vec::new())
                    } else if let Some(git) = &entry.git_url {
                        let clean_git = git
                            .strip_prefix("https://github.com/")
                            .unwrap_or(git)
                            .strip_suffix(".git")
                            .unwrap_or(git)
                            .to_string();
                        (None, vec![clean_git])
                    } else {
                        let mut resolved_pkgs = Vec::new();
                        for pkg in &entry.packages {
                            if let Some(sub) = parsed.packages.get(pkg) {
                                if let Some(git) = &sub.git_url {
                                    let clean_git = git
                                        .strip_prefix("https://github.com/")
                                        .unwrap_or(git)
                                        .strip_suffix(".git")
                                        .unwrap_or(git)
                                        .to_string();
                                    resolved_pkgs.push(clean_git);
                                } else {
                                    resolved_pkgs.push(pkg.clone());
                                }
                            } else {
                                resolved_pkgs.push(pkg.clone());
                            }
                        }
                        (None, resolved_pkgs)
                    };
                    aliases.insert(
                        name.clone(),
                        CuratedAlias {
                            description: entry.description.clone(),
                            bundle,
                            packages,
                            default: entry.default,
                            recommend_boost: entry.recommend_boost,
                        },
                    );
                }
                if let Ok(embedded) = serde_json::from_str::<CuratedRegistryFile>(EMBEDDED_REGISTRY)
                {
                    for (alias, entry) in embedded.aliases {
                        aliases.entry(alias).or_insert(entry);
                    }
                }
                return Ok(CuratedRegistryFile {
                    version: parsed.version,
                    aliases,
                });
            }
        }
    }

    let raw = registry_sync::read_registry_file(&home, "packages.json", EMBEDDED_REGISTRY);
    let mut reg: CuratedRegistryFile =
        serde_json::from_str(&raw).context("parse package registry")?;

    // Robustness guarantee: always merge embedded aliases so built-ins (@matt-skills, etc.)
    // are never dropped even if a stale or incomplete cached file exists on disk.
    if let Ok(embedded) = serde_json::from_str::<CuratedRegistryFile>(EMBEDDED_REGISTRY) {
        for (alias, entry) in embedded.aliases {
            reg.aliases.entry(alias).or_insert(entry);
        }
    }
    Ok(reg)
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
            default: entry.default,
            recommend_boost: entry.recommend_boost,
        })
        .collect())
}

/// Git sources marked `default: true` in the curated registry (deduped).
pub fn default_git_sources() -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in list_aliases()? {
        if !entry.default || entry.bundle.is_some() {
            continue;
        }
        for src in entry.packages {
            if !out.iter().any(|s| s == &src) {
                out.push(src);
            }
        }
    }
    Ok(out)
}

/// Max `recommend_boost` per git source from the curated registry.
pub fn recommend_boost_by_source() -> Result<std::collections::HashMap<String, f64>> {
    let mut map = std::collections::HashMap::new();
    for entry in list_aliases()? {
        if entry.recommend_boost <= 0.0 {
            continue;
        }
        if let Some(bundle) = &entry.bundle {
            let e = map.entry(bundle.clone()).or_insert(0.0);
            if entry.recommend_boost > *e {
                *e = entry.recommend_boost;
            }
            let reg_key = format!("registry:packages/{bundle}");
            let e = map.entry(reg_key).or_insert(0.0);
            if entry.recommend_boost > *e {
                *e = entry.recommend_boost;
            }
            let bndl_key = format!("agent-brain/bundle:{bundle}");
            let e = map.entry(bndl_key).or_insert(0.0);
            if entry.recommend_boost > *e {
                *e = entry.recommend_boost;
            }
        }
        for src in entry.packages {
            let e = map.entry(src).or_insert(0.0);
            if entry.recommend_boost > *e {
                *e = entry.recommend_boost;
            }
        }
    }
    Ok(map)
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
            bail!("empty package alias; use @matt-skills, @starter, @supervisor, @nextjs, @ecc, or owner/repo");
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
        assert!(sources.iter().any(|s| s.contains("mattpocock/skills")));
        assert!(sources
            .iter()
            .any(|s| s.contains("vercel-labs/agent-skills")));
    }

    #[test]
    fn resolves_matt_skills_as_default_with_boost() {
        let sources = resolve_package_inputs("@matt-skills").unwrap();
        assert_eq!(sources, vec!["mattpocock/skills"]);
        let aliases = list_aliases().unwrap();
        let matt = aliases.iter().find(|a| a.alias == "matt-skills").unwrap();
        assert!(matt.default);
        assert!(matt.recommend_boost >= 0.3);
        let defaults = default_git_sources().unwrap();
        assert!(defaults.iter().any(|s| s == "mattpocock/skills"));
    }

    #[test]
    fn passes_through_owner_repo() {
        let sources = resolve_package_inputs("foo/bar").unwrap();
        assert_eq!(sources, vec!["foo/bar"]);
    }

    #[test]
    fn resolves_autonomic_core_bundle_alias() {
        let resolved = lookup_alias("@autonomic-core").unwrap().unwrap();
        assert_eq!(resolved, ResolvedAlias::Bundle("autonomic-core".into()));
    }

    #[test]
    fn embedded_registry_lists_workflows() {
        let workflows = list_workflows().unwrap();
        assert!(workflows.iter().any(|w| w.alias == "engineering-safety"));
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
    fn utilities_catalog_includes_agent_body() {
        let utils = list_utilities().unwrap();
        assert!(utils.iter().any(|u| u.name == "agent-body"));
        assert!(utils.iter().any(|u| u.name == "agent-heart"));
        assert!(utils.iter().any(|u| u.name == "agent-muscle"));
        let body = utils.iter().find(|u| u.name == "agent-body").unwrap();
        assert!(
            body.description
                .to_ascii_lowercase()
                .contains("control plane")
                || body.description.to_ascii_lowercase().contains("prefer")
        );
    }

    #[test]
    fn resolves_agstack_bundle_alias() {
        let resolved = lookup_alias("@agstack").unwrap().unwrap();
        assert_eq!(resolved, ResolvedAlias::Bundle("agstack".into()));
    }
}
