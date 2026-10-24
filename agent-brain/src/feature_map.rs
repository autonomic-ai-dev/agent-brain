use std::fs;
use std::path::Path;

const CODE_HEADING: &str = "## Code";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureMap {
    pub file: String,
    pub code: Vec<String>,
}

pub fn load(features_dir: &Path) -> Vec<FeatureMap> {
    let Ok(entries) = fs::read_dir(features_dir) else {
        return Vec::new();
    };
    let mut maps: Vec<FeatureMap> = entries
        .filter_map(|entry| {
            let file = entry.ok()?.file_name().into_string().ok()?;
            if !file.ends_with(".md") || file == "README.md" {
                return None;
            }
            let text = fs::read_to_string(features_dir.join(&file)).ok()?;
            Some(FeatureMap {
                code: parse_code_section(&text),
                file,
            })
        })
        .collect();
    maps.sort_by(|a, b| a.file.cmp(&b.file));
    maps
}

pub fn parse_code_section(text: &str) -> Vec<String> {
    let mut in_code = false;
    let mut paths = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            in_code = line.trim_end() == CODE_HEADING;
        } else if in_code {
            if let Some(path) = line
                .strip_prefix("- `")
                .and_then(|rest| rest.strip_suffix('`'))
            {
                paths.push(path.trim_end_matches('/').to_string());
            }
        }
    }
    paths
}

fn is_code(path: &str) -> bool {
    !path.starts_with(".verify/") && !path.ends_with(".md")
}

fn owns(entry: &str, path: &str) -> bool {
    path == entry || path.strip_prefix(entry).is_some_and(|rest| rest.starts_with('/'))
}

fn owners<'a>(features: &'a [FeatureMap], path: &str) -> Vec<&'a FeatureMap> {
    features
        .iter()
        .filter(|feature| feature.code.iter().any(|entry| owns(entry, path)))
        .collect()
}

pub fn gap(changed: &[String], features: &[FeatureMap], knowledge: &[String]) -> Option<String> {
    let code: Vec<&str> = changed
        .iter()
        .map(String::as_str)
        .filter(|path| is_code(path))
        .collect();
    if code.is_empty() {
        return None;
    }

    let unclaimed: Vec<&str> = code
        .iter()
        .copied()
        .filter(|path| owners(features, path).is_empty())
        .collect();
    let mut parts = Vec::new();
    if !unclaimed.is_empty() {
        parts.push(format!(
            "Code changed outside every feature's `## Code` section: {}. Add each file or directory to the `## Code` section of the feature that owns it, or create a feature map with a `## Code` section and a knowledge-base entry for it. Then finish again.",
            quoted(&unclaimed)
        ));
    }

    let mut owned: Vec<&FeatureMap> = Vec::new();
    for path in &code {
        for feature in owners(features, path) {
            if !owned.iter().any(|known| known.file == feature.file) {
                owned.push(feature);
            }
        }
    }
    let incomplete: Vec<&str> = owned
        .iter()
        .filter(|feature| {
            !changed.contains(&format!(".verify/features/{}", feature.file))
                || !changed.contains(&format!(".verify/knowledge-base/{}", feature.file))
        })
        .map(|feature| feature.file.as_str())
        .collect();
    if !incomplete.is_empty() {
        parts.push(format!(
            "These features own changed code, but their feature map and knowledge-base entry were not both updated in this turn: {}. Update `.verify/features/<name>` and `.verify/knowledge-base/<name>` for each, then finish again.",
            quoted(&incomplete)
        ));
    }

    let missing_entry: Vec<&str> = features
        .iter()
        .filter(|feature| !knowledge.contains(&feature.file))
        .map(|feature| feature.file.as_str())
        .collect();
    if !missing_entry.is_empty() {
        parts.push(format!(
            "These features have no knowledge-base entry: {}. Create `.verify/knowledge-base/<name>` for each, then finish again.",
            quoted(&missing_entry)
        ));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn quoted(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn feature(file: &str, code: &[&str]) -> FeatureMap {
        FeatureMap {
            file: file.to_string(),
            code: names(code),
        }
    }

    #[test]
    fn parses_only_the_bullets_under_the_code_heading() {
        let text = "# gate\n\n## Sub-features\n\n- `not/code.rs`\n\n## Code\n\n- `agent-x/src/a.rs`\n- `agent-x/src/b/`\n- plain prose\n\n## Gotchas\n\n- `also/not/code.rs`\n";
        assert_eq!(
            parse_code_section(text),
            vec!["agent-x/src/a.rs".to_string(), "agent-x/src/b".to_string()]
        );
    }

    #[test]
    fn an_entry_owns_the_file_it_names_and_files_under_it() {
        assert!(owns("agent-x/src/a.rs", "agent-x/src/a.rs"));
        assert!(owns("agent-x/src", "agent-x/src/a.rs"));
        assert!(!owns("agent-x/src/a", "agent-x/src/ab.rs"));
    }

    #[test]
    fn docs_and_verify_changes_are_not_code() {
        let changed = names(&["README.md", ".verify/verify.toml"]);
        assert_eq!(gap(&changed, &[], &[]), None);
    }

    #[test]
    fn code_no_feature_claims_is_reported_as_unclaimed() {
        let changed = names(&["agent-x/src/new.rs"]);
        let features = [feature("gate.md", &["agent-x/src/a.rs"])];
        let reason = gap(&changed, &features, &names(&["gate.md"])).unwrap();
        assert!(reason.contains("`agent-x/src/new.rs`"));
        assert!(reason.contains("Add each file or directory"));
    }

    #[test]
    fn owned_code_needs_its_feature_map_and_entry_changed() {
        let features = [feature("gate.md", &["agent-x/src/a.rs"])];
        let knowledge = names(&["gate.md"]);
        let map_only = names(&["agent-x/src/a.rs", ".verify/features/gate.md"]);
        assert!(gap(&map_only, &features, &knowledge)
            .unwrap()
            .contains("`gate.md`"));
        let entry_only = names(&["agent-x/src/a.rs", ".verify/knowledge-base/gate.md"]);
        assert!(gap(&entry_only, &features, &knowledge).is_some());
    }

    #[test]
    fn owned_code_with_both_updates_is_complete() {
        let changed = names(&[
            "agent-x/src/a.rs",
            ".verify/features/gate.md",
            ".verify/knowledge-base/gate.md",
        ]);
        let features = [feature("gate.md", &["agent-x/src/a.rs"])];
        assert_eq!(gap(&changed, &features, &names(&["gate.md"])), None);
    }

    #[test]
    fn a_new_feature_that_claims_new_code_is_complete_with_its_entry() {
        let changed = names(&[
            "agent-x/src/new.rs",
            ".verify/features/new.md",
            ".verify/knowledge-base/new.md",
        ]);
        let features = [
            feature("gate.md", &["agent-x/src/a.rs"]),
            feature("new.md", &["agent-x/src/new.rs"]),
        ];
        let knowledge = names(&["gate.md", "new.md"]);
        assert_eq!(gap(&changed, &features, &knowledge), None);
    }

    #[test]
    fn a_feature_without_a_knowledge_entry_on_disk_is_reported() {
        let changed = names(&[
            "agent-x/src/a.rs",
            ".verify/features/gate.md",
            ".verify/knowledge-base/gate.md",
        ]);
        let features = [feature("gate.md", &["agent-x/src/a.rs"])];
        let reason = gap(&changed, &features, &[]).unwrap();
        assert!(reason.contains("no knowledge-base entry"));
    }

    #[test]
    fn a_shared_file_requires_every_owning_feature_to_be_updated() {
        let features = [
            feature("one.md", &["agent-x/src/shared.rs"]),
            feature("two.md", &["agent-x/src"]),
        ];
        let knowledge = names(&["one.md", "two.md"]);
        let changed = names(&[
            "agent-x/src/shared.rs",
            ".verify/features/one.md",
            ".verify/knowledge-base/one.md",
        ]);
        let reason = gap(&changed, &features, &knowledge).unwrap();
        assert!(reason.contains("`two.md`"));
        assert!(!reason.contains("`one.md`"));
    }
}
