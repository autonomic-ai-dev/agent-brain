use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::store::{content_hash, looks_like_secret, BrainStore};
use crate::embed::Embedder;
use crate::lessons::Lesson;
use crate::secrets::SecretRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncSource {
    ManualImport,
    Git,
    Cloud,
}

impl SyncSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualImport => "manual_import",
            Self::Git => "git",
            Self::Cloud => "cloud",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePolicy {
    NewerWins,
    KeepLocal,
    KeepRemote,
}

impl MergePolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "newer_wins" => Some(Self::NewerWins),
            "keep_local" => Some(Self::KeepLocal),
            "keep_remote" => Some(Self::KeepRemote),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub imported: usize,
    pub deduplicated: usize,
    pub conflicts_resolved: usize,
    pub skipped: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleManifest {
    schema_version: u32,
    device_id: String,
    exported_at: i64,
    fact_count: usize,
    includes_vectors: bool,
    checksums: BundleChecksums,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleChecksums {
    facts_jsonl: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lessons_jsonl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleFact {
    pub id: String,
    pub topic: String,
    pub fact: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub source: String,
    pub confidence: f64,
    pub polarity: String,
    #[serde(default)]
    pub apply_when: Option<Vec<String>>,
    pub content_hash: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn export_bundle(store: &BrainStore, home: &Path, dest: Option<&Path>) -> Result<PathBuf> {
    write_bundle(store, home, dest, false)
}

pub fn export_bundle_with_lessons(
    store: &BrainStore,
    home: &Path,
    dest: Option<&Path>,
) -> Result<PathBuf> {
    write_bundle(store, home, dest, true)
}

fn write_bundle(
    store: &BrainStore,
    home: &Path,
    dest: Option<&Path>,
    include_lessons: bool,
) -> Result<PathBuf> {
    let facts = store.list_export_facts()?;
    let facts_body = if facts.is_empty() {
        String::new()
    } else {
        facts
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?
            .join("\n")
            + "\n"
    };
    let facts_checksum = sha256_hex(facts_body.as_bytes());
    let lessons_body = if include_lessons {
        let now = chrono::Utc::now().timestamp_millis();
        Some(lessons_jsonl(&store.lessons_all(now)?))
    } else {
        None
    };

    let device_id = store.ensure_device_id()?;
    let manifest = BundleManifest {
        schema_version: 1,
        device_id,
        exported_at: chrono::Utc::now().timestamp_millis(),
        fact_count: facts.len(),
        includes_vectors: false,
        checksums: BundleChecksums {
            facts_jsonl: facts_checksum,
            lessons_jsonl: lessons_body.as_ref().map(|body| sha256_hex(body.as_bytes())),
        },
    };

    let out_dir = dest.map(PathBuf::from).unwrap_or_else(|| {
        home.join("export")
            .join(format!("sync-bundle-{}", chrono::Utc::now().timestamp()))
    });
    fs::create_dir_all(&out_dir).context("create bundle dir")?;

    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    fs::write(out_dir.join("facts.jsonl"), facts_body)?;
    if let Some(body) = &lessons_body {
        fs::write(out_dir.join("lessons.jsonl"), body)?;
    }
    let secret_refs = store.list_secret_refs()?;
    fs::write(
        out_dir.join("secret_refs.json"),
        serde_json::to_string_pretty(&secret_refs)?,
    )?;

    Ok(out_dir)
}

pub fn import_bundle(
    store: &BrainStore,
    embedder: &Embedder,
    bundle_path: &Path,
    policy: MergePolicy,
    sync_source: SyncSource,
) -> Result<ImportReport> {
    let manifest_raw =
        fs::read_to_string(bundle_path.join("manifest.json")).context("read manifest.json")?;
    let schema_version = serde_json::from_str::<serde_json::Value>(&manifest_raw)
        .ok()
        .and_then(|v| v.get("schema_version").and_then(|x| x.as_u64()))
        .unwrap_or(1);
    if schema_version >= 2 {
        let min_reader = serde_json::from_str::<serde_json::Value>(&manifest_raw)
            .ok()
            .and_then(|v| v.get("min_reader_version").and_then(|x| x.as_u64()))
            .unwrap_or(2);
        if min_reader > 2 {
            anyhow::bail!(
                "bundle min_reader_version {min_reader} is newer than this tekmor (supports 2)"
            );
        }
        return crate::sync::v2::import_v2_bundle(
            store,
            embedder,
            bundle_path,
            policy,
            sync_source,
        );
    }
    let manifest: BundleManifest =
        serde_json::from_str(&manifest_raw).context("parse manifest.json")?;
    let facts_path = bundle_path.join("facts.jsonl");
    if !facts_path.is_file() {
        // v1 manifest without facts — nothing to do
        return Ok(ImportReport::default());
    }
    let facts_body = fs::read_to_string(&facts_path).context("read facts.jsonl")?;
    let checksum = sha256_hex(facts_body.as_bytes());
    if checksum != manifest.checksums.facts_jsonl {
        anyhow::bail!("facts.jsonl checksum mismatch");
    }

    let mut report = ImportReport::default();
    for line in facts_body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let remote: BundleFact = serde_json::from_str(line).context("parse facts.jsonl line")?;
        import_one_fact(store, embedder, &remote, policy, sync_source, &mut report)?;
    }

    import_lessons(
        store,
        bundle_path,
        manifest.checksums.lessons_jsonl.as_deref(),
        &mut report,
    )?;

    let refs_path = bundle_path.join("secret_refs.json");
    if refs_path.is_file() {
        let raw = fs::read_to_string(&refs_path).context("read secret_refs.json")?;
        let refs: Vec<SecretRef> = serde_json::from_str(&raw).context("parse secret_refs.json")?;
        store.merge_secret_refs(&refs)?;
    }

    Ok(report)
}

fn import_one_fact(
    store: &BrainStore,
    embedder: &Embedder,
    remote: &BundleFact,
    policy: MergePolicy,
    sync_source: SyncSource,
    report: &mut ImportReport,
) -> Result<()> {
    let hash = if remote.content_hash.is_empty() {
        content_hash(&remote.fact)
    } else {
        remote.content_hash.clone()
    };

    if store.fact_exists_by_hash(&hash, &remote.scope, remote.scope_key.as_deref())? {
        report.deduplicated += 1;
        return Ok(());
    }

    let local = store.get_active_fact_by_topic(
        &remote.topic,
        &remote.scope,
        remote.scope_key.as_deref(),
    )?;

    if let Some(local) = &local {
        match policy {
            MergePolicy::KeepLocal => {
                report.skipped += 1;
                return Ok(());
            }
            MergePolicy::NewerWins if local.updated_at >= remote.updated_at => {
                report.skipped += 1;
                return Ok(());
            }
            MergePolicy::KeepRemote | MergePolicy::NewerWins => {}
        }
    }

    let apply_when_json = remote
        .apply_when
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let embedding = embedder.embed_one(&format!("{} {}", remote.topic, remote.fact))?;
    let res = store.store_fact_full(
        &remote.topic,
        &remote.fact,
        &remote.scope,
        remote.scope_key.as_deref(),
        remote.confidence,
        &remote.source,
        &hash,
        &embedding,
        &remote.polarity,
        apply_when_json.as_deref(),
        None,
    )?;

    if let Some(local) = &local {
        match policy {
            MergePolicy::KeepRemote | MergePolicy::NewerWins => {
                store.log_import_conflict(
                    sync_source.as_str(),
                    &remote.topic,
                    &remote.scope,
                    remote.scope_key.as_deref(),
                    &local.id,
                    &local.fact,
                    &res.id,
                    &remote.fact,
                )?;
                report.conflicts_resolved += 1;
            }
            _ => {}
        }
    }

    if res.stored {
        report.imported += 1;
    } else if res.deduplicated {
        report.deduplicated += 1;
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleLesson {
    id: String,
    project_key: String,
    paths: Vec<String>,
    cmd: String,
    excerpt: String,
    created_at: i64,
}

impl From<&Lesson> for BundleLesson {
    fn from(lesson: &Lesson) -> Self {
        Self {
            id: lesson.id.clone(),
            project_key: lesson.project_key.clone(),
            paths: lesson.paths.clone(),
            cmd: lesson.cmd.clone(),
            excerpt: lesson.excerpt.clone(),
            created_at: lesson.created_at,
        }
    }
}

impl From<BundleLesson> for Lesson {
    fn from(lesson: BundleLesson) -> Self {
        Self {
            id: lesson.id,
            project_key: lesson.project_key,
            paths: lesson.paths,
            cmd: lesson.cmd,
            excerpt: lesson.excerpt,
            created_at: lesson.created_at,
        }
    }
}

fn names_a_secret(lesson: &BundleLesson) -> bool {
    looks_like_secret(&lesson.cmd)
        || looks_like_secret(&lesson.excerpt)
        || lesson.paths.iter().any(|path| looks_like_secret(path))
}

fn lessons_jsonl(lessons: &[Lesson]) -> String {
    lessons
        .iter()
        .map(BundleLesson::from)
        .filter(|lesson| !names_a_secret(lesson))
        .filter_map(|lesson| serde_json::to_string(&lesson).ok())
        .map(|line| line + "\n")
        .collect()
}

fn accepted_lessons(body: &str) -> Result<(Vec<Lesson>, usize)> {
    let mut accepted = Vec::new();
    let mut refused = 0;
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let lesson: BundleLesson = serde_json::from_str(line).context("parse lessons.jsonl line")?;
        if names_a_secret(&lesson) {
            refused += 1;
        } else {
            accepted.push(Lesson::from(lesson));
        }
    }
    Ok((accepted, refused))
}

fn import_lessons(
    store: &BrainStore,
    bundle_path: &Path,
    expected_checksum: Option<&str>,
    report: &mut ImportReport,
) -> Result<()> {
    let path = bundle_path.join("lessons.jsonl");
    if !path.is_file() {
        return Ok(());
    }
    let body = fs::read_to_string(&path).context("read lessons.jsonl")?;
    let Some(expected) = expected_checksum else {
        anyhow::bail!("lessons.jsonl is not covered by the manifest checksum");
    };
    if sha256_hex(body.as_bytes()) != expected {
        anyhow::bail!("lessons.jsonl checksum mismatch");
    }
    let (lessons, refused) = accepted_lessons(&body)?;
    for lesson in &lessons {
        store.insert_lesson(lesson)?;
    }
    report.imported += lessons.len();
    report.skipped += refused;
    Ok(())
}

#[cfg(test)]
mod lesson_bundle_tests {
    use super::*;

    fn lesson(id: &str, cmd: &str, excerpt: &str) -> Lesson {
        Lesson {
            id: id.into(),
            project_key: "p".into(),
            paths: vec!["src/a.rs".into()],
            cmd: cmd.into(),
            excerpt: excerpt.into(),
            created_at: 1,
        }
    }

    #[test]
    fn a_lesson_with_a_secret_looking_excerpt_is_left_out_of_the_bundle() {
        let body = lessons_jsonl(&[
            lesson("clean", "cargo test", "assert failed"),
            lesson("leaky", "cargo test", "password=hunter2"),
        ]);
        let (accepted, refused) = accepted_lessons(&body).unwrap();
        assert_eq!(refused, 0);
        assert_eq!(accepted.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["clean"]);
    }

    #[test]
    fn a_secret_looking_lesson_written_by_hand_is_refused_on_import() {
        let body = serde_json::to_string(&BundleLesson::from(&lesson(
            "leaky",
            "cargo test",
            "token: abc123",
        )))
        .unwrap();
        let (accepted, refused) = accepted_lessons(&(body + "\n")).unwrap();
        assert!(accepted.is_empty());
        assert_eq!(refused, 1);
    }

    #[test]
    fn a_lesson_that_names_an_env_file_is_refused() {
        let mut leaky = lesson("env", "cargo test", "assert failed");
        leaky.paths = vec![".env".into()];
        assert!(lessons_jsonl(&[leaky]).is_empty());
    }
}

#[cfg(test)]
mod lesson_import_store_tests {
    use super::*;

    fn bundle_with_lessons(dir: &Path, body: &str) {
        fs::write(dir.join("lessons.jsonl"), body).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "device_id": "test",
            "exported_at": 0,
            "fact_count": 0,
            "includes_vectors": false,
            "checksums": { "facts_jsonl": "", "lessons_jsonl": sha256_hex(body.as_bytes()) }
        });
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }

    fn lesson_line(id: &str, cmd: &str, excerpt: &str, created_at: i64) -> String {
        serde_json::to_string(&BundleLesson {
            id: id.into(),
            project_key: "p".into(),
            paths: vec!["src/a.rs".into()],
            cmd: cmd.into(),
            excerpt: excerpt.into(),
            created_at,
        })
        .unwrap()
            + "\n"
    }

    #[test]
    fn an_imported_lesson_is_stored_and_a_secret_looking_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let body = lesson_line("clean", "cargo test", "assert failed", now)
            + &lesson_line("leaky", "cargo test", "password=hunter2", now);
        let bundle = tempfile::tempdir().unwrap();
        bundle_with_lessons(bundle.path(), &body);

        let mut report = ImportReport::default();
        let expected = sha256_hex(body.as_bytes());
        import_lessons(&store, bundle.path(), Some(&expected), &mut report).unwrap();

        let stored: Vec<String> = store
            .lessons_for_project("p", now)
            .unwrap()
            .into_iter()
            .map(|l| l.id)
            .collect();
        assert_eq!(stored, ["clean"]);
        assert_eq!((report.imported, report.skipped), (1, 1));
    }

    #[test]
    fn a_lessons_file_with_a_wrong_checksum_is_refused_and_nothing_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("brain.db")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let body = lesson_line("clean", "cargo test", "assert failed", now);
        let bundle = tempfile::tempdir().unwrap();
        bundle_with_lessons(bundle.path(), &body);

        let mut report = ImportReport::default();
        let result = import_lessons(&store, bundle.path(), Some("deadbeef"), &mut report);
        assert!(result.is_err());
        assert!(store.lessons_for_project("p", now).unwrap().is_empty());
    }
}
