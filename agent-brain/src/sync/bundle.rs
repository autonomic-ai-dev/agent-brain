use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::store::{content_hash, BrainStore};
use crate::embed::Embedder;
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

    let device_id = store.ensure_device_id()?;
    let manifest = BundleManifest {
        schema_version: 1,
        device_id,
        exported_at: chrono::Utc::now().timestamp_millis(),
        fact_count: facts.len(),
        includes_vectors: false,
        checksums: BundleChecksums {
            facts_jsonl: facts_checksum,
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
                "bundle min_reader_version {min_reader} is newer than this agent-brain (supports 2)"
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
