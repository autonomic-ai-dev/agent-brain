use std::fs;
use std::path::Path;

use anyhow::{bail, Result};

pub const MAX_SKILL_BYTES: u64 = 256 * 1024;
pub const MAX_SKILL_RESULTS: usize = 100;

pub fn search_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect()
}

pub fn read_skill_file(path: &Path) -> Result<String> {
    let len = fs::metadata(path)?.len();
    if len > MAX_SKILL_BYTES {
        bail!("skill file is {len} bytes, above the {MAX_SKILL_BYTES} byte limit");
    }
    Ok(fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::{BrainStore, IndexBatchItem};
    use crate::types::ItemType;
    use tempfile::TempDir;

    fn indexed(item_type: ItemType, topic: &str, text: &str, source_path: &str) -> IndexBatchItem {
        IndexBatchItem {
            id: format!("{}-{topic}", item_type.as_str()),
            item_type,
            topic: topic.to_string(),
            text: text.to_string(),
            source_path: source_path.to_string(),
            scope: "global".to_string(),
            scope_key: None,
            content_hash: format!("hash-{topic}"),
            embedding: Vec::new(),
            file_mtime: None,
        }
    }

    fn seeded_store(dir: &TempDir) -> BrainStore {
        let store = BrainStore::open(&dir.path().join("brain.db")).expect("open store");
        let items = vec![
            indexed(
                ItemType::Skill,
                "rust-patterns",
                "rust-patterns Idiomatic Rust ownership and error handling",
                "/skills/rust-patterns/SKILL.md",
            ),
            indexed(
                ItemType::Skill,
                "react-patterns",
                "react-patterns Hooks and component composition",
                "/skills/react-patterns/SKILL.md",
            ),
            indexed(
                ItemType::Rule,
                "rust-style",
                "rust-style Rule about rust formatting",
                "/rules/rust-style.md",
            ),
        ];
        store
            .upsert_indexed_items_batch(&items)
            .expect("seed indexed items");
        store
    }

    #[test]
    fn splits_query_into_lowercase_terms() {
        assert_eq!(
            search_terms("  Rust  OWNERSHIP "),
            vec!["rust".to_string(), "ownership".to_string()]
        );
        assert!(search_terms("   ").is_empty());
    }

    #[test]
    fn finds_skills_matching_every_term_and_skips_other_item_types() {
        let dir = TempDir::new().expect("tempdir");
        let store = seeded_store(&dir);

        let rows = store
            .find_skill_items(&search_terms("rust ownership"), 10)
            .expect("search");
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, vec!["rust-patterns"]);
        assert_eq!(rows[0].path, "/skills/rust-patterns/SKILL.md");

        let rules_hit = store
            .find_skill_items(&search_terms("formatting"), 10)
            .expect("search");
        assert!(
            rules_hit.is_empty(),
            "rules must not appear in skill search"
        );
    }

    #[test]
    fn treats_like_wildcards_in_query_as_plain_text() {
        let dir = TempDir::new().expect("tempdir");
        let store = seeded_store(&dir);

        assert!(store
            .find_skill_items(&search_terms("%"), 10)
            .expect("search")
            .is_empty());
        assert!(store
            .find_skill_items(&search_terms("rust_ownership"), 10)
            .expect("search")
            .is_empty());
    }

    #[test]
    fn respects_result_limit() {
        let dir = TempDir::new().expect("tempdir");
        let store = seeded_store(&dir);

        let rows = store
            .find_skill_items(&search_terms("patterns"), 1)
            .expect("search");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn resolves_skill_name_case_insensitively_and_rejects_unknown_names() {
        let dir = TempDir::new().expect("tempdir");
        let store = seeded_store(&dir);

        assert_eq!(
            store.skill_source_path("REACT-PATTERNS").expect("lookup"),
            Some("/skills/react-patterns/SKILL.md".to_string())
        );
        assert_eq!(store.skill_source_path("rust-style").expect("lookup"), None);
        assert_eq!(store.skill_source_path("missing").expect("lookup"), None);
    }

    #[test]
    fn reads_skill_file_within_limit() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("SKILL.md");
        fs::write(&path, "# Skill\nBody").expect("write skill");

        assert_eq!(read_skill_file(&path).expect("read"), "# Skill\nBody");
    }

    #[test]
    fn rejects_skill_file_above_size_limit() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("SKILL.md");
        fs::write(&path, vec![b'a'; MAX_SKILL_BYTES as usize + 1]).expect("write skill");

        assert!(read_skill_file(&path).is_err());
    }

    #[test]
    fn reports_missing_skill_file() {
        let dir = TempDir::new().expect("tempdir");

        assert!(read_skill_file(&dir.path().join("gone.md")).is_err());
    }
}
