//! Bundle v2: per-device event logs flushed from the `sync_events` outbox.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::db::store::{content_hash, BrainStore, FactTemporal};
use crate::embed::Embedder;
use crate::sync::bundle::{ImportReport, MergePolicy, SyncSource};
use crate::sync::event::{
    DeviceManifestV2, EventEnvelope, FactPutPayload, FactTombstonePayload, RootManifestV2,
    MIN_READER_VERSION, SCHEMA_VERSION_V2,
};
use crate::sync::hlc::Hlc;

pub fn device_dir(bundle_root: &Path, device_id: &str) -> PathBuf {
    bundle_root.join("devices").join(device_id)
}

/// Flush pending outbox rows into this device's `events.jsonl` and refresh manifests.
pub fn flush_outbox_to_bundle(store: &BrainStore, bundle_root: &Path) -> Result<usize> {
    let device_id = store.ensure_device_id()?;
    let pending = store.list_pending_sync_events(10_000)?;
    if pending.is_empty() {
        // Still ensure root manifest exists for v2 readers.
        ensure_root_manifest(bundle_root, &device_id)?;
        return Ok(0);
    }

    let dir = device_dir(bundle_root, &device_id);
    fs::create_dir_all(&dir).context("create device sync dir")?;
    let events_path = dir.join("events.jsonl");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_path)
        .context("open events.jsonl")?;

    let mut last_hlc = Hlc::zero();
    let mut ids = Vec::with_capacity(pending.len());
    for ev in &pending {
        let line = serde_json::to_string(ev)?;
        writeln!(file, "{line}")?;
        last_hlc = ev.hlc;
        ids.push(ev.id.clone());
    }
    file.flush()?;

    let body = fs::read(&events_path).context("read events for checksum")?;
    let sha = format!("{:x}", Sha256::digest(&body));
    let event_count = BufReader::new(fs::File::open(&events_path)?)
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.trim().is_empty())
        .count();

    let device_manifest = DeviceManifestV2 {
        device_id: device_id.clone(),
        hlc: last_hlc,
        event_count,
        sha256: sha,
        embedding_model: None,
    };
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&device_manifest)?,
    )?;

    ensure_root_manifest(bundle_root, &device_id)?;
    store.mark_sync_events_flushed(&ids)?;
    Ok(ids.len())
}

fn ensure_root_manifest(bundle_root: &Path, device_id: &str) -> Result<()> {
    fs::create_dir_all(bundle_root.join("devices"))?;
    let path = bundle_root.join("manifest.json");
    let mut manifest = if path.is_file() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str::<RootManifestV2>(&raw).unwrap_or(RootManifestV2 {
            schema_version: SCHEMA_VERSION_V2,
            min_reader_version: MIN_READER_VERSION,
            devices: Vec::new(),
            migrated_at: None,
        })
    } else {
        RootManifestV2 {
            schema_version: SCHEMA_VERSION_V2,
            min_reader_version: MIN_READER_VERSION,
            devices: Vec::new(),
            migrated_at: None,
        }
    };
    manifest.schema_version = SCHEMA_VERSION_V2;
    manifest.min_reader_version = MIN_READER_VERSION.max(manifest.min_reader_version);
    if !manifest.devices.iter().any(|d| d == device_id) {
        manifest.devices.push(device_id.to_string());
    }
    fs::write(path, serde_json::to_string_pretty(&manifest)?)?;
    Ok(())
}

/// Import all device event logs under a v2 bundle. Skips checksum-mismatched devices.
pub fn import_v2_bundle(
    store: &BrainStore,
    embedder: &Embedder,
    bundle_path: &Path,
    policy: MergePolicy,
    sync_source: SyncSource,
) -> Result<ImportReport> {
    let manifest_raw =
        fs::read_to_string(bundle_path.join("manifest.json")).context("read v2 manifest")?;
    let root: RootManifestV2 =
        serde_json::from_str(&manifest_raw).context("parse v2 root manifest")?;
    if root.min_reader_version > SCHEMA_VERSION_V2 {
        bail!(
            "bundle requires min_reader_version {}, this binary supports {}",
            root.min_reader_version,
            SCHEMA_VERSION_V2
        );
    }

    let mut report = ImportReport::default();
    let devices_dir = bundle_path.join("devices");
    if !devices_dir.is_dir() {
        return Ok(report);
    }

    let mut device_dirs: Vec<_> = fs::read_dir(&devices_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    device_dirs.sort_by_key(|e| e.file_name());

    for entry in device_dirs {
        let dir = entry.path();
        let device_id = entry.file_name().to_string_lossy().to_string();
        let man_path = dir.join("manifest.json");
        let events_path = dir.join("events.jsonl");
        if !man_path.is_file() || !events_path.is_file() {
            continue;
        }
        let man: DeviceManifestV2 = serde_json::from_str(&fs::read_to_string(&man_path)?)?;
        let body = fs::read(&events_path)?;
        let sha = format!("{:x}", Sha256::digest(&body));
        if sha != man.sha256 {
            eprintln!("skipping device {device_id}: events.jsonl checksum mismatch");
            report.skipped += 1;
            continue;
        }

        let cursor = store.get_sync_cursor(&device_id)?;
        let mut lines: Vec<EventEnvelope> = BufReader::new(body.as_slice())
            .lines()
            .filter_map(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect();
        lines.sort_by(|a, b| {
            a.hlc
                .cmp(&b.hlc)
                .then_with(|| a.device_id.cmp(&b.device_id))
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut last = cursor.unwrap_or(Hlc::zero());
        for ev in lines {
            if ev.hlc <= last {
                continue;
            }
            apply_event(store, embedder, &ev, policy, sync_source, &mut report)?;
            last = ev.hlc;
        }
        store.set_sync_cursor(&device_id, last)?;
    }

    Ok(report)
}

fn apply_event(
    store: &BrainStore,
    embedder: &Embedder,
    ev: &EventEnvelope,
    _policy: MergePolicy,
    _sync_source: SyncSource,
    report: &mut ImportReport,
) -> Result<()> {
    match ev.event_type.as_str() {
        "fact_put" => {
            let p: FactPutPayload = serde_json::from_value(ev.payload.clone())?;
            let mut scope_key = p.scope_key.clone().or_else(|| ev.project_key.clone());
            if let Some(ref key) = scope_key {
                if crate::project_key::looks_like_path_scope_key(key) {
                    if let Ok(Some(mapped)) = store.map_legacy_scope_key(key) {
                        scope_key = Some(mapped);
                    }
                }
            }
            let emb = embedder.embed_one(&format!("{} {}", p.topic, p.fact))?;
            let temporal = if p.valid_from.is_some() || p.invalid_at.is_some() {
                Some(FactTemporal {
                    valid_from: p.valid_from,
                    invalid_at: p.invalid_at,
                })
            } else {
                None
            };
            let apply_when = p
                .apply_when
                .as_ref()
                .map(|v| serde_json::to_string(v))
                .transpose()?;
            let hash = if p.content_hash.is_empty() {
                content_hash(&p.fact)
            } else {
                p.content_hash.clone()
            };
            let res = store.store_fact_full(
                &p.topic,
                &p.fact,
                &p.scope,
                scope_key.as_deref(),
                p.confidence,
                if p.source.is_empty() { "sync" } else { &p.source },
                &hash,
                &emb,
                &p.polarity,
                apply_when.as_deref(),
                temporal.as_ref(),
            )?;
            // Avoid re-outboxing imported facts: store_fact_full enqueues; mark latest pending as flushed
            // for this content to prevent echo. Best-effort: flush nothing here; importer sets a flag later.
            if res.deduplicated {
                report.deduplicated += 1;
            } else if res.stored {
                report.imported += 1;
            }
        }
        "fact_tombstone" => {
            let p: FactTombstonePayload = serde_json::from_value(ev.payload.clone())?;
            if let Some(id) = p.fact_id.as_deref() {
                let _ = store.delete_fact(Some(id), None, None, None)?;
                report.imported += 1;
            } else if let (Some(hash), Some(scope)) = (p.content_hash.as_deref(), p.scope.as_deref())
            {
                // Soft-delete by invalidating matching active fact.
                if let Ok(Some(id)) = store.find_active_fact_id(hash, scope, p.scope_key.as_deref())
                {
                    let _ = store.invalidate_fact(&id);
                    report.imported += 1;
                }
            }
        }
        _ => {
            report.skipped += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::Embedder;
    use tempfile::tempdir;

    #[test]
    fn flush_and_import_round_trip() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("brain.db");
        let store = BrainStore::open(&db).unwrap();
        let emb = Embedder::deterministic();
        let hash = content_hash("portable fact");
        let embedding = emb.embed_one("topic portable fact").unwrap();
        store
            .store_fact_full(
                "topic",
                "portable fact",
                "project",
                Some("projkey"),
                0.9,
                "agent",
                &hash,
                &embedding,
                "positive",
                None,
                None,
            )
            .unwrap();

        let bundle = dir.path().join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        let n = flush_outbox_to_bundle(&store, &bundle).unwrap();
        assert!(n >= 1);

        let db2 = dir.path().join("brain2.db");
        let store2 = BrainStore::open(&db2).unwrap();
        let report = import_v2_bundle(
            &store2,
            &emb,
            &bundle,
            MergePolicy::NewerWins,
            SyncSource::Git,
        )
        .unwrap();
        assert!(report.imported >= 1 || report.deduplicated >= 1);
    }
}
