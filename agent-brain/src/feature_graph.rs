use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Frontmatter {
    feature: String,
    #[serde(default)]
    related: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct FeatureNode {
    pub id: String,
    pub sub_features: Vec<String>,
    pub code: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct FeatureEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct FeatureGraph {
    pub nodes: Vec<FeatureNode>,
    pub edges: Vec<FeatureEdge>,
}

pub fn build(dir: &Path) -> Result<FeatureGraph> {
    let mut maps = feature_maps(dir)?;
    maps.sort();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for path in maps {
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let stem = file_stem(&path)?;
        let (front, body) =
            split_frontmatter(&text).with_context(|| format!("{}", path.display()))?;
        let front: Frontmatter = serde_yaml::from_str(front)
            .with_context(|| format!("frontmatter of {}", path.display()))?;
        if front.feature != stem {
            bail!(
                "{}: feature '{}' must match the file name '{stem}'",
                path.display(),
                front.feature
            );
        }
        for target in &front.related {
            edges.push(FeatureEdge {
                from: stem.clone(),
                to: wiki_target(target)?,
            });
        }
        nodes.push(FeatureNode {
            id: stem,
            sub_features: sub_features(body),
            code: code_paths(body),
        });
    }
    let ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    if let Some(edge) = edges.iter().find(|edge| !ids.contains(edge.to.as_str())) {
        bail!("{} links to unknown feature {}", edge.from, edge.to);
    }
    Ok(FeatureGraph { nodes, edges })
}

fn feature_maps(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
    let mut maps = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let is_map = path.extension().is_some_and(|ext| ext == "md");
        if is_map && path.file_name().is_some_and(|name| name != "README.md") {
            maps.push(path);
        }
    }
    Ok(maps)
}

fn file_stem(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("file name of {} is not UTF-8", path.display()))?;
    Ok(stem.to_string())
}

fn split_frontmatter(text: &str) -> Result<(&str, &str)> {
    let rest = text.strip_prefix("---\n").context("missing frontmatter")?;
    let end = rest.find("\n---\n").context("unterminated frontmatter")?;
    Ok((&rest[..end], &rest[end + "\n---\n".len()..]))
}

fn wiki_target(link: &str) -> Result<String> {
    let name = link
        .strip_prefix("[[")
        .and_then(|rest| rest.strip_suffix("]]"))
        .with_context(|| format!("related entry '{link}' must be a [[wikilink]]"))?;
    Ok(name.to_string())
}

fn sub_features(body: &str) -> Vec<String> {
    section_items(body, "Sub-features")
}

fn code_paths(body: &str) -> Vec<String> {
    section_items(body, "Code")
}

fn section_items(body: &str, heading: &str) -> Vec<String> {
    let mut inside = false;
    let mut items = Vec::new();
    for line in body.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            inside = title == heading;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(item) = line
            .strip_prefix("- `")
            .and_then(|rest| rest.split('`').next())
        {
            items.push(item.to_string());
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_map(dir: &Path, name: &str, text: &str) {
        fs::write(dir.join(format!("{name}.md")), text).expect("write map");
    }

    const CLI_MAP: &str = "---\nfeature: cli\nrelated:\n  - \"[[core]]\"\n---\n\n# cli\n\n## Sub-features\n\n- `verify-record-exit`: Runs a command.\n\n## Code\n\n- `agent-brain/src`\n- `agent-brain/src/main.rs` (`serve` branch)\n\n## Gotchas\n";
    const CORE_MAP: &str = "---\nfeature: core\n---\n\n# core\n\n## Sub-features\n\n- `ranking`: Orders candidates.\n\n## Code\n\n- `agent-brain/crates/agent-brain-core`\n";

    #[test]
    fn builds_nodes_sub_features_code_and_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_map(dir.path(), "cli", CLI_MAP);
        write_map(dir.path(), "core", CORE_MAP);

        let graph = build(dir.path()).expect("graph");

        assert_eq!(graph.nodes[0].id, "cli");
        assert_eq!(graph.nodes[0].sub_features, vec!["verify-record-exit"]);
        assert_eq!(
            graph.nodes[0].code,
            vec!["agent-brain/src", "agent-brain/src/main.rs"]
        );
        assert_eq!(graph.nodes[1].id, "core");
        assert_eq!(
            graph.edges,
            vec![FeatureEdge {
                from: "cli".into(),
                to: "core".into()
            }]
        );
    }

    #[test]
    fn rejects_link_to_unknown_feature() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_map(dir.path(), "cli", CLI_MAP);

        let err = build(dir.path()).expect_err("dangling link");

        assert!(err.to_string().contains("unknown feature core"));
    }

    #[test]
    fn rejects_feature_name_that_differs_from_file_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_map(
            dir.path(),
            "core",
            CLI_MAP.replace("feature: cli", "feature: other").as_str(),
        );

        let err = build(dir.path()).expect_err("name mismatch");

        assert!(err.to_string().contains("must match the file name"));
    }

    #[test]
    fn rejects_map_without_frontmatter() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_map(dir.path(), "core", "# core\n\n## Code\n");

        let err = build(dir.path()).expect_err("missing frontmatter");

        assert!(format!("{err:#}").contains("missing frontmatter"));
    }

    #[test]
    fn repository_feature_maps_form_a_valid_graph() {
        let maps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.verify/features");

        let graph = build(&maps).expect("repository feature maps");

        assert!(graph.nodes.len() >= 4);
        assert!(graph.nodes.iter().all(|node| !node.code.is_empty()));
        assert!(!graph.edges.is_empty());
    }
}
