use agent_brain::db::store::{content_hash, BrainStore};
use agent_brain::recall::{rank, CANDIDATE_CAP};
use tempfile::tempdir;

fn insert(store: &BrainStore, topic: &str, fact: &str) {
    store
        .store_fact(
            topic,
            fact,
            "project",
            None,
            0.9,
            "cli",
            &content_hash(fact),
            &[],
            "neutral",
        )
        .unwrap();
}

fn recall(store: &BrainStore, tags: &[&str], query: Option<&str>, limit: usize) -> Vec<String> {
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    let rows = store
        .recall_rows(&tags, chrono::Utc::now().timestamp_millis(), CANDIDATE_CAP)
        .unwrap();
    rank(rows, query, limit)
        .into_iter()
        .map(|item| item.text)
        .collect()
}

#[test]
fn empty_store_returns_an_empty_list() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    assert!(recall(&store, &["voice"], None, 20).is_empty());
    assert!(recall(&store, &[], Some("anything"), 20).is_empty());
}

#[test]
fn tag_miss_returns_nothing() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    insert(&store, "posted", "Shipped the release notes tool");
    assert!(recall(&store, &["voice"], None, 20).is_empty());
}

#[test]
fn tag_filter_keeps_only_the_requested_topic() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    insert(&store, "voice", "I write short, plain sentences");
    insert(&store, "posted", "Shipped the release notes tool");
    assert_eq!(
        recall(&store, &["voice"], None, 20),
        vec!["I write short, plain sentences"]
    );
}

#[test]
fn limit_caps_the_result_count() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    for i in 0..5 {
        insert(&store, "voice", &format!("sample post number {i}"));
    }
    assert_eq!(recall(&store, &["voice"], None, 2).len(), 2);
    assert_eq!(recall(&store, &["voice"], Some("sample post"), 3).len(), 3);
}

#[test]
fn query_only_returns_facts_that_share_a_token() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    insert(&store, "voice", "plain sentences about rust builds");
    insert(&store, "voice", "gardening notes");
    assert_eq!(
        recall(&store, &["voice"], Some("rust builds"), 20),
        vec!["plain sentences about rust builds"]
    );
}
