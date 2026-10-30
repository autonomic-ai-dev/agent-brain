use agent_brain::db::store::{BrainStore, RecallRow};
use agent_brain::embed::deterministic_embedding;
use agent_brain::recall::{rank, CANDIDATE_CAP};
use agent_brain::remember::{import_corpus, remember, RememberRequest};
use tempfile::tempdir;

fn embed(text: &str) -> anyhow::Result<Vec<f32>> {
    Ok(deterministic_embedding(text))
}

fn request(text: &str, tags: &[&str], source: &str) -> RememberRequest {
    RememberRequest {
        text: text.to_string(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        source: source.to_string(),
    }
}

fn rows(store: &BrainStore, tags: &[&str]) -> Vec<RecallRow> {
    let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
    store
        .recall_rows(&tags, chrono::Utc::now().timestamp_millis(), CANDIDATE_CAP)
        .unwrap()
}

#[test]
fn remember_then_recall_returns_the_same_text_and_tags() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    let outcome = remember(
        &store,
        &request(
            "I write short sentences.",
            &["voice", "posted"],
            "https://example.com/p1",
        ),
        &embed,
    )
    .unwrap();
    assert!(!outcome.deduplicated);

    let items = rank(rows(&store, &["voice"]), None, 20);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, outcome.id);
    assert_eq!(items[0].text, "I write short sentences.");
    assert_eq!(
        items[0].tags,
        vec!["posted".to_string(), "voice".to_string()]
    );
    assert_eq!(items[0].source, "https://example.com/p1");
}

#[test]
fn a_fact_is_found_by_any_of_its_tags() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    remember(
        &store,
        &request("Shipped the release notes tool.", &["posted"], "sha:abc123"),
        &embed,
    )
    .unwrap();
    assert_eq!(rows(&store, &["posted"]).len(), 1);
    assert!(rows(&store, &["voice"]).is_empty());
}

#[test]
fn remembering_the_same_text_again_is_a_dedupe() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    let first = remember(
        &store,
        &request("Same post text.", &["posted"], "sha:1"),
        &embed,
    )
    .unwrap();
    let second = remember(
        &store,
        &request("Same post text.", &["posted"], "sha:1"),
        &embed,
    )
    .unwrap();
    assert_eq!(first.id, second.id);
    assert!(second.deduplicated);
    assert_eq!(rows(&store, &["posted"]).len(), 1);
}

#[test]
fn secrets_are_refused_and_nothing_is_stored() {
    let dir = tempdir().unwrap();
    let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
    let result = remember(
        &store,
        &request("api_key = sk-live-123456", &["voice"], "cli"),
        &embed,
    );
    assert!(result.is_err());
    assert!(rows(&store, &["voice"]).is_empty());
}

#[test]
fn importing_the_same_folder_twice_creates_no_new_facts() {
    let corpus = tempdir().unwrap();
    std::fs::write(
        corpus.path().join("post-one.md"),
        "First paragraph about shipping.\n\nSecond paragraph about tests.\n",
    )
    .unwrap();
    std::fs::write(corpus.path().join("notes.txt"), "not markdown, ignored").unwrap();

    let db = tempdir().unwrap();
    let store = BrainStore::open(&db.path().join("brain.db")).unwrap();
    let tags = vec!["voice".to_string()];

    let first = import_corpus(&store, corpus.path(), &tags, &embed).unwrap();
    assert_eq!(first.files, 1);
    assert_eq!(first.paragraphs, 2);
    assert_eq!(first.stored, 2);
    assert_eq!(first.deduplicated, 0);

    let second = import_corpus(&store, corpus.path(), &tags, &embed).unwrap();
    assert_eq!(second.stored, 0);
    assert_eq!(second.deduplicated, 2);
    assert_eq!(rows(&store, &["voice"]).len(), 2);
}

#[test]
fn imported_facts_keep_a_source_that_points_at_the_original_file() {
    let corpus = tempdir().unwrap();
    let file = corpus.path().join("post.md");
    std::fs::write(&file, "A single paragraph.\n").unwrap();

    let db = tempdir().unwrap();
    let store = BrainStore::open(&db.path().join("brain.db")).unwrap();
    import_corpus(&store, corpus.path(), &["voice".to_string()], &embed).unwrap();

    let stored = rows(&store, &["voice"]);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].source, format!("{}#paragraph-1", file.display()));
}

#[test]
fn import_skips_secret_paragraphs_and_counts_them() {
    let corpus = tempdir().unwrap();
    std::fs::write(
        corpus.path().join("mixed.md"),
        "A safe paragraph.\n\npassword = hunter2-long-value\n",
    )
    .unwrap();

    let db = tempdir().unwrap();
    let store = BrainStore::open(&db.path().join("brain.db")).unwrap();
    let report = import_corpus(&store, corpus.path(), &["voice".to_string()], &embed).unwrap();
    assert_eq!(report.stored, 1);
    assert_eq!(report.rejected_secret, 1);
    assert_eq!(rows(&store, &["voice"]).len(), 1);
}

#[test]
fn import_needs_a_tag_and_a_directory() {
    let db = tempdir().unwrap();
    let store = BrainStore::open(&db.path().join("brain.db")).unwrap();
    assert!(import_corpus(&store, db.path(), &[], &embed).is_err());
    assert!(import_corpus(
        &store,
        &db.path().join("missing"),
        &["voice".to_string()],
        &embed
    )
    .is_err());
}
