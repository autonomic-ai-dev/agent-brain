use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use anyhow::Result;

use crate::cache::TurnCache;
use crate::db::store::{content_hash, looks_like_secret, word_count, BrainStore};
use crate::db::write_queue::{store_memory_payload, WriteOp};
use crate::embed::Embedder;
use crate::sync::{ImportReport, MergePolicy, SyncSource};

pub struct WriteHandlerCtx {
    pub store: Arc<BrainStore>,
    pub embedder: Arc<Embedder>,
    pub cache: Arc<TurnCache>,
    pub home: PathBuf,
    pub auto_capture: bool,
}

impl WriteHandlerCtx {
    pub fn handle(&self, op: WriteOp) {
        match op {
            WriteOp::StoreMemory { resp_tx, payload } => {
                let _ = resp_tx.send(self.handle_store_memory(payload));
            }
            WriteOp::DeleteMemory {
                resp_tx,
                id,
                topic,
                scope,
                scope_key,
            } => {
                let result = self
                    .store
                    .delete_fact(
                        id.as_deref(),
                        topic.as_deref(),
                        scope.as_deref(),
                        scope_key.as_deref(),
                    )
                    .map(|n| serde_json::json!({ "deleted": n }));
                let _ = resp_tx.send(result);
            }
            WriteOp::ImportBundle {
                resp_tx,
                bundle_path,
                policy,
                source,
            } => {
                let _ = resp_tx.send(self.handle_import_bundle(bundle_path, policy, source));
            }
            WriteOp::ReindexComplete => {}
        }
    }

    fn handle_store_memory(
        &self,
        payload: store_memory_payload::StoreMemoryRequest,
    ) -> Result<serde_json::Value> {
        if !self.auto_capture {
            anyhow::bail!("auto capture disabled");
        }
        if looks_like_secret(&payload.fact) {
            anyhow::bail!("prohibited content");
        }
        if word_count(&payload.fact) > 50 {
            anyhow::bail!("fact exceeds 50 words");
        }
        let hash = content_hash(&payload.fact);
        let embedding = self
            .embedder
            .embed_one(&format!("{} {}", payload.topic, payload.fact))?;
        let polarity = payload.polarity.as_deref().unwrap_or("positive");
        let apply_when = payload
            .apply_when
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let temporal = if payload.valid_from.is_some() || payload.invalid_at.is_some() {
            Some(crate::db::store::FactTemporal {
                valid_from: payload.valid_from,
                invalid_at: payload.invalid_at,
            })
        } else {
            None
        };
        let res = self.store.store_fact_full(
            &payload.topic,
            &payload.fact,
            &payload.scope,
            payload.scope_key.as_deref(),
            payload.confidence,
            "agent",
            &hash,
            &embedding,
            polarity,
            apply_when.as_deref(),
            temporal.as_ref(),
        )?;
        self.cache.clear();
        self.store.bump_index_version().ok();

        if res.stored {
            let settings = crate::settings::AgentBrainSettings::load(&self.home);
            if settings.observation.enabled {
                let cfg = crate::observation::ObservationConfig {
                    min_facts_per_topic: settings.observation.min_facts_per_topic,
                    window_days: settings.observation.window_days,
                };
                if let Err(err) =
                    crate::observation::run_observations(&self.store, &self.embedder, &cfg, false)
                {
                    tracing::warn!(target: "agent_brain::observation", "run failed: {err}");
                }
            }
            if settings.trace_extract.enabled {
                let cfg = crate::trace_extract::TraceExtractConfig {
                    confidence: settings.trace_extract.confidence,
                    explain: false,
                };
                if let Err(err) = crate::trace_extract::run_trace_extract(
                    &self.store,
                    &self.embedder,
                    &self.home,
                    &cfg,
                    false,
                ) {
                    tracing::warn!(target: "agent_brain::trace_extract", "run failed: {err}");
                }
            }
            if settings.sync.git.auto_push {
                if let Err(err) = crate::sync::git_push(&self.store, &self.home, &settings.sync.git)
                {
                    tracing::warn!(target: "agent_brain::sync", "git auto_push failed: {err}");
                }
            }
            if settings.sync.cloud.enabled && settings.sync.cloud.auto_push {
                if let Err(err) =
                    crate::sync::cloud_push(&self.store, &self.home, &settings.sync.cloud)
                {
                    tracing::warn!(target: "agent_brain::sync", "cloud auto_push failed: {err}");
                }
            }
        }

        Ok(serde_json::json!({
            "id": res.id,
            "stored": res.stored,
            "deduplicated": res.deduplicated
        }))
    }

    fn handle_import_bundle(
        &self,
        bundle_path: PathBuf,
        policy: MergePolicy,
        source: SyncSource,
    ) -> Result<ImportReport> {
        let report = crate::sync::import_bundle(
            &self.store,
            &self.embedder,
            &bundle_path,
            policy,
            source,
        )?;
        self.cache.clear();
        self.store.bump_index_version().ok();
        Ok(report)
    }
}

pub fn spawn_write_handler(
    store: Arc<BrainStore>,
    embedder: Arc<Embedder>,
    cache: Arc<TurnCache>,
    home: PathBuf,
    auto_capture: bool,
) -> crate::db::WriteQueue {
    crate::db::WriteQueue::spawn(move |op| {
        let ctx = WriteHandlerCtx {
            store: Arc::clone(&store),
            embedder: Arc::clone(&embedder),
            cache: Arc::clone(&cache),
            home: home.clone(),
            auto_capture,
        };
        ctx.handle(op);
    })
}

pub fn send_and_recv<T>(
    queue: &crate::db::WriteQueue,
    build: impl FnOnce(Sender<Result<T>>) -> WriteOp,
) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    queue.send(build(tx))?;
    rx.recv()
        .map_err(|e| anyhow::anyhow!("write queue response channel closed: {e}"))?
}
