use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::cache::{
    fingerprint_query, route_cache_key, session_route_cache_key, QueryEmbeddingCache, TurnCache,
};
use crate::config::Config;
use crate::db::store::{content_hash, BrainStore};
use crate::db::{send_and_recv, spawn_write_handler, WriteOp, WriteQueue};
use crate::db::{RouteLatencyStats, RouteTiming};
use crate::embed::{parse_embedding_model, Embedder};
use crate::index;
use crate::mcp_activity::McpActivity;
use crate::route_briefing;
use crate::sync::{ImportReport, MergePolicy, SyncSource};
use crate::tokens::estimate_json_tokens;
use crate::types::{
    AgentRec, GetContextItem, GetContextResponse, ItemType, MemoryRec, MustApply, RouteLimits,
    RouteTaskResponse, RuleRec, ScoredItem, SkillRec,
};
use crate::workspace::{
    agent_boost_keywords, infer_phase, is_low_signal_memory, mcp_route_expansion_tags, probe,
};

type RouteQueryParallelResult = (Vec<ScoredItem>, usize, usize, u64, u64, bool, bool);

pub struct Engine {
    pub config: Config,
    pub store: Arc<BrainStore>,
    pub embedder: Arc<Embedder>,
    pub cache: Arc<TurnCache>,
    pub auto_capture_enabled: bool,
    pub route_latency: Arc<RouteLatencyStats>,
    pub warmed: Arc<AtomicBool>,
    pub query_emb_cache: Arc<QueryEmbeddingCache>,
    pub mcp_activity: Arc<McpActivity>,
    prev_briefing: Mutex<Option<String>>,
    prev_snapshot: Mutex<Option<ResponseSnapshot>>,
    touched_files: Mutex<Vec<String>>,
    prev_tool_names: Mutex<Vec<String>>,
    write_queue: WriteQueue,
}

impl Engine {
    pub fn new(config: Config) -> Result<Self> {
        config.ensure_dirs()?;
        let store = Arc::new(BrainStore::open(&config.db_path)?);
        let embed_model = parse_embedding_model(&config.embedding_model);
        let embedder = Arc::new(Embedder::with_model(embed_model)?);
        if store.ensure_embedding_model(embedder.model_id)? {
            tracing::info!(
                target: "agent_brain::index",
                model = embedder.model_id,
                "run index to refresh embeddings after model change"
            );
        }
        let cache = Arc::new(TurnCache::new(64, config.turn_ttl_secs));
        let write_queue = spawn_write_handler(
            Arc::clone(&store),
            Arc::clone(&embedder),
            Arc::clone(&cache),
            config.home.clone(),
            config.auto_capture_enabled,
        );
        Ok(Self {
            auto_capture_enabled: config.auto_capture_enabled,
            config,
            store,
            embedder,
            cache,
            route_latency: Arc::new(RouteLatencyStats::new(256)),
            warmed: Arc::new(AtomicBool::new(false)),
            query_emb_cache: Arc::new(QueryEmbeddingCache::new(128)),
            mcp_activity: Arc::new(McpActivity::new()),
            prev_briefing: Mutex::new(None),
            prev_snapshot: Mutex::new(None),
            touched_files: Mutex::new(Vec::new()),
            prev_tool_names: Mutex::new(Vec::new()),
            write_queue,
        })
    }

    /// Pre-opened store for integration tests (same wiring as [`Self::new`]).
    ///
    /// Uses [`Embedder::deterministic`] so parallel `cargo test` runs do not contend on
    /// fastembed ONNX cache locks and stored test vectors stay in the same embedding space.
    #[doc(hidden)]
    pub fn new_with_store(config: Config, store: Arc<BrainStore>) -> Result<Self> {
        Self::new_with_store_and_embedder(config, store, Arc::new(Embedder::deterministic()))
    }

    #[doc(hidden)]
    pub fn new_with_store_and_embedder(
        config: Config,
        store: Arc<BrainStore>,
        embedder: Arc<Embedder>,
    ) -> Result<Self> {
        let cache = Arc::new(TurnCache::new(64, config.turn_ttl_secs));
        let write_queue = spawn_write_handler(
            Arc::clone(&store),
            Arc::clone(&embedder),
            Arc::clone(&cache),
            config.home.clone(),
            config.auto_capture_enabled,
        );
        Ok(Self {
            auto_capture_enabled: config.auto_capture_enabled,
            config,
            store,
            embedder,
            cache,
            route_latency: Arc::new(RouteLatencyStats::new(256)),
            warmed: Arc::new(AtomicBool::new(false)),
            query_emb_cache: Arc::new(QueryEmbeddingCache::new(128)),
            mcp_activity: Arc::new(McpActivity::new()),
            prev_briefing: Mutex::new(None),
            prev_snapshot: Mutex::new(None),
            touched_files: Mutex::new(Vec::new()),
            prev_tool_names: Mutex::new(Vec::new()),
            write_queue,
        })
    }

    pub fn write_queue(&self) -> &WriteQueue {
        &self.write_queue
    }

    pub fn import_bundle_queued(
        &self,
        bundle: &Path,
        policy: MergePolicy,
        source: SyncSource,
    ) -> Result<ImportReport> {
        let bundle_path = bundle.to_path_buf();
        send_and_recv(self.write_queue(), |resp_tx| WriteOp::ImportBundle {
            resp_tx,
            bundle_path,
            policy,
            source,
        })
    }

    pub fn bootstrap(self: &Arc<Self>, cwd: Option<&Path>) -> Result<usize> {
        let mut n =
            index::sync_index_opts(&self.store, &self.config, &self.embedder, cwd, false, true)?;
        if self.config.session_ingest_enabled && !self.config.session_ingest_background {
            let sessions =
                crate::sessions::ingest_sessions(&self.store, &self.embedder, &self.config)?;
            if sessions > 0 {
                tracing::info!("session ingest: imported {sessions} session facts");
                // Digests are low-signal — do not invalidate turn/session caches.
            }
            n += sessions;
        }
        if self.config.prewarm_on_bootstrap {
            if let Err(err) = self.prewarm() {
                tracing::warn!(error = %err, "bootstrap prewarm failed");
            }
        }
        let settings = crate::settings::AgentBrainSettings::load(&self.config.home);
        if settings.upstream_mcp.enabled {
            match crate::upstream::refresh_upstream_index_blocking(
                &settings.upstream_mcp,
                &self.store,
            ) {
                Ok(count) if count > 0 => {
                    tracing::info!(count, "upstream MCP tools indexed");
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "upstream MCP index refresh failed"),
            }
        }
        let now = chrono::Utc::now().timestamp().to_string();
        if let Err(err) = self.store.set_meta("last_bootstrap_unix", &now) {
            tracing::warn!(error = %err, "record last_bootstrap_unix failed");
        }
        Ok(n)
    }

    pub fn spawn_auto_update(self: &Arc<Self>) {
        let settings = crate::settings::AgentBrainSettings::load(&self.config.home);
        if !settings.auto_update.enabled {
            return;
        }
        let initial_delay = self.config.auto_update_startup_delay_secs;
        let recheck_minutes = settings.auto_update.mcp.recheck_interval_minutes;
        let engine = Arc::clone(self);
        std::thread::spawn(move || {
            let mut delay = initial_delay;
            loop {
                if delay > 0 {
                    std::thread::sleep(Duration::from_secs(delay));
                }
                match crate::auto_update::run(
                    &engine,
                    &settings,
                    false,
                    false,
                    crate::auto_update::AutoUpdateRunOptions::background_serve(),
                ) {
                    Ok(report) if report.mcp_updated || report.packages_updated > 0 => {
                        tracing::info!(
                            target: "agent_brain::auto_update",
                            packages = report.packages_updated,
                            mcp = report.mcp_updated,
                            reindexed = report.reindexed,
                            "auto-update applied"
                        );
                    }
                    Ok(_) => {}
                    Err(err) => tracing::warn!(error = %err, "auto-update failed"),
                }
                if recheck_minutes == 0 {
                    break;
                }
                delay = 0;
                std::thread::sleep(Duration::from_secs(recheck_minutes * 60));
            }
        });
    }

    pub fn spawn_session_ingest(self: &Arc<Self>) {
        if !self.config.session_ingest_enabled || !self.config.session_ingest_background {
            return;
        }
        let delay = self.config.session_ingest_delay_secs;
        let engine = Arc::clone(self);
        std::thread::spawn(move || {
            if delay > 0 {
                std::thread::sleep(Duration::from_secs(delay));
            }
            if let Ok(sessions) = engine.run_session_ingest_sync() {
                if sessions > 0 {
                    tracing::info!(
                        target: "agent_brain::sessions",
                        sessions,
                        "background session ingest complete"
                    );
                }
            }
        });
    }

    pub fn run_session_ingest_sync(&self) -> Result<usize> {
        if !self.config.session_ingest_enabled {
            return Ok(0);
        }
        let sessions = crate::sessions::ingest_sessions(&self.store, &self.embedder, &self.config)?;
        // Digests are low-signal — do not invalidate turn/session caches.
        let now = chrono::Utc::now().timestamp().to_string();
        let _ = self.store.set_meta("last_session_ingest_unix", &now);
        Ok(sessions)
    }

    fn session_ingest_due(&self) -> bool {
        let interval = self.config.session_ingest_route_interval_secs;
        if interval == 0 {
            return false;
        }
        let last = self
            .store
            .get_meta("last_session_ingest_unix")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        chrono::Utc::now().timestamp() - last >= interval as i64
    }

    /// Background session ingest when stale — triggered from route_task so MCP use refreshes cross-agent digests.
    pub fn maybe_spawn_session_ingest(&self) {
        if !self.config.session_ingest_enabled || !self.session_ingest_due() {
            return;
        }
        let store = Arc::clone(&self.store);
        let embedder = Arc::clone(&self.embedder);
        let config = self.config.clone();
        std::thread::spawn(move || {
            match crate::sessions::ingest_sessions(&store, &embedder, &config) {
                Ok(sessions) if sessions > 0 => {
                    // Digests are low-signal — do not invalidate turn/session caches.
                    let now = chrono::Utc::now().timestamp().to_string();
                    let _ = store.set_meta("last_session_ingest_unix", &now);
                    tracing::info!(
                        target: "agent_brain::sessions",
                        sessions,
                        "route-triggered session ingest complete"
                    );
                }
                Ok(_) => {
                    let now = chrono::Utc::now().timestamp().to_string();
                    let _ = store.set_meta("last_session_ingest_unix", &now);
                }
                Err(err) => tracing::warn!(error = %err, "route-triggered session ingest failed"),
            }
        });
    }

    /// Index skills/rules and ingest session digests — run after install or doctor --fix.
    pub fn post_install_warmup(&self) -> Result<(usize, usize)> {
        let indexed =
            index::sync_index_opts(&self.store, &self.config, &self.embedder, None, false, true)?;
        let sessions = self.run_session_ingest_sync()?;
        Ok((indexed, sessions))
    }

    fn bootstrap_due(&self) -> bool {
        let interval = self.config.bootstrap_interval_secs;
        if interval == 0 {
            return true;
        }
        let last = self
            .store
            .get_meta("last_bootstrap_unix")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let elapsed = chrono::Utc::now().timestamp() - last;
        elapsed >= interval as i64
    }

    pub fn spawn_bootstrap(self: &Arc<Self>, cwd: Option<&Path>) {
        let engine = Arc::clone(self);
        let cwd = cwd.map(|p| p.to_path_buf());
        let startup_delay = engine.config.bootstrap_startup_delay_secs;
        std::thread::spawn(move || {
            if startup_delay > 0 {
                std::thread::sleep(Duration::from_secs(startup_delay));
            }
            if !engine.bootstrap_due() {
                tracing::info!(
                    target: "agent_brain::bootstrap",
                    interval_secs = engine.config.bootstrap_interval_secs,
                    "skipped (indexed recently)"
                );
                if engine.config.session_ingest_enabled && engine.config.session_ingest_background {
                    engine.spawn_session_ingest();
                }
                return;
            }
            let cwd_ref = cwd.as_deref();
            match engine.bootstrap(cwd_ref) {
                Ok(n) => {
                    tracing::info!(target: "agent_brain::bootstrap", items = n, "bootstrap complete")
                }
                Err(err) => tracing::warn!(error = %err, "background bootstrap failed"),
            }
            if engine.config.session_ingest_enabled && engine.config.session_ingest_background {
                engine.spawn_session_ingest();
            }
        });
    }

    pub fn prewarm(&self) -> Result<()> {
        self.store.prewarm_search_cache()?;
        self.embedder.embed_one("agent-brain warmup")?;
        self.warmed.store(true, Ordering::Relaxed);
        tracing::info!(target: "agent_brain::route", "bootstrap prewarm complete");
        Ok(())
    }

    fn embed_query(&self, query: &str, message_fp: Option<&str>) -> Result<(Vec<f32>, bool)> {
        let query_hash = content_hash(query);

        for key in std::iter::once(query_hash.as_str()).chain(message_fp) {
            if let Some(embedding) = self.query_emb_cache.get(key) {
                if key != query_hash.as_str() {
                    self.query_emb_cache
                        .put(query_hash.clone(), embedding.clone());
                }
                return Ok((embedding, true));
            }
        }

        if self.config.embedding_cache_enabled {
            if let Some(embedding) = self.store.get_query_embedding(&query_hash)? {
                self.query_emb_cache
                    .put(query_hash.clone(), embedding.clone());
                if let Some(fp) = message_fp {
                    self.query_emb_cache.put(fp.to_string(), embedding.clone());
                }
                return Ok((embedding, true));
            }
            let embedding = self.embedder.embed_one(query)?;
            self.store.put_query_embedding(&query_hash, &embedding)?;
            self.query_emb_cache
                .put(query_hash.clone(), embedding.clone());
            if let Some(fp) = message_fp {
                self.query_emb_cache.put(fp.to_string(), embedding.clone());
            }
            return Ok((embedding, false));
        }

        let embedding = self.embedder.embed_one(query)?;
        self.query_emb_cache
            .put(query_hash.clone(), embedding.clone());
        if let Some(fp) = message_fp {
            self.query_emb_cache.put(fp.to_string(), embedding.clone());
        }
        Ok((embedding, false))
    }

    #[allow(clippy::too_many_arguments)]
    fn route_query_parallel(
        &self,
        query: &str,
        message_fp: &str,
        repo_root: Option<&str>,
        tags: &[String],
        boost_agents: bool,
        phase: &str,
        open_files: &[String],
        user_message: &str,
    ) -> Result<RouteQueryParallelResult> {
        let query_owned = query.to_string();
        let store = Arc::clone(&self.store);
        let bm25_handle = std::thread::spawn(move || store.bm25_prefilter(&query_owned));

        let bm25 = bm25_handle
            .join()
            .map_err(|_| anyhow::anyhow!("bm25 prefilter thread panicked"))??;

        let use_bm25_fast_path =
            self.config.bm25_fast_path_enabled && bm25.fast_path_eligible(query);

        let (query_emb, embed_cache_hit, embed_us) = if use_bm25_fast_path {
            (Vec::new(), false, 0)
        } else {
            let embed_started = Instant::now();
            let (emb, hit) = self.embed_query(query, Some(message_fp))?;
            (emb, hit, embed_started.elapsed().as_micros() as u64)
        };

        let score_started = Instant::now();
        let snapshot = self.store.search_cache_snapshot()?;
        let match_ctx = crate::intelligence::MatchContext {
            phase,
            tags,
            open_files,
            repo_root,
            user_message,
        };
        let package_boosts = crate::packages::recommend_path_boosts(&self.config.home);
        let (mut scored, candidates, index_total) = self.store.score_items_with_bm25(
            &snapshot,
            query,
            &query_emb,
            &bm25,
            repo_root,
            tags,
            boost_agents,
            use_bm25_fast_path,
            Some(phase),
            Some(&match_ctx),
            &package_boosts,
            crate::ann::AnnSettings {
                enabled: self.config.ann_enabled,
                min_index: self.config.ann_min_index,
                top_k: self.config.ann_top_k,
            },
        )?;

        // Precision layer (Phase 9+): opt-in only. Default off so skills.sh Recall@3 stays ≥ 0.85.
        // Enable with AGENT_BRAIN_QUERY_DECOMPOSE=1 / AGENT_BRAIN_PRECISION_RERANK=1 after eval gate.
        if self.config.query_decompose_enabled {
            scored = self.apply_query_decompose_fusion(
                user_message,
                scored,
                repo_root,
                tags,
                boost_agents,
                phase,
                open_files,
                use_bm25_fast_path,
            )?;
        }
        if self.config.precision_rerank_enabled {
            crate::cross_encoder_rerank::rerank_scored_items(
                user_message,
                &mut scored,
                crate::cross_encoder_rerank::DEFAULT_TOP_K,
            );
        }

        let score_us = score_started.elapsed().as_micros() as u64;

        Ok((
            scored,
            candidates,
            index_total,
            embed_us,
            score_us,
            embed_cache_hit,
            use_bm25_fast_path,
        ))
    }

    /// RRF-fuse primary rankings with up to two extra sub-query rankings.
    fn apply_query_decompose_fusion(
        &self,
        user_message: &str,
        primary: Vec<crate::types::ScoredItem>,
        repo_root: Option<&str>,
        tags: &[String],
        boost_agents: bool,
        phase: &str,
        open_files: &[String],
        use_bm25_fast_path: bool,
    ) -> anyhow::Result<Vec<crate::types::ScoredItem>> {
        let parts = crate::query_decompose::decompose_query(user_message);
        if parts.len() <= 1 {
            return Ok(primary);
        }

        let mut by_id: std::collections::HashMap<String, crate::types::ScoredItem> =
            primary.iter().cloned().map(|s| (s.id.clone(), s)).collect();
        let mut rankings: Vec<Vec<(String, f64)>> =
            vec![primary.iter().map(|s| (s.id.clone(), s.score)).collect()];

        let snapshot = self.store.search_cache_snapshot()?;
        let match_ctx = crate::intelligence::MatchContext {
            phase,
            tags,
            open_files,
            repo_root,
            user_message,
        };
        let package_boosts = crate::packages::recommend_path_boosts(&self.config.home);
        let ann = crate::ann::AnnSettings {
            enabled: self.config.ann_enabled,
            min_index: self.config.ann_min_index,
            top_k: self.config.ann_top_k,
        };

        for part in parts.iter().skip(1).take(2) {
            let part_query = format!("{} {}", part, tags.join(" "));
            let bm25 = self.store.bm25_prefilter(&part_query)?;
            let query_emb = if use_bm25_fast_path {
                Vec::new()
            } else {
                self.embed_query(&part_query, None)?.0
            };
            let (extra, _, _) = self.store.score_items_with_bm25(
                &snapshot,
                &part_query,
                &query_emb,
                &bm25,
                repo_root,
                tags,
                boost_agents,
                use_bm25_fast_path,
                Some(phase),
                Some(&match_ctx),
                &package_boosts,
                ann,
            )?;
            for item in &extra {
                by_id.entry(item.id.clone()).or_insert_with(|| item.clone());
            }
            rankings.push(extra.iter().map(|s| (s.id.clone(), s.score)).collect());
        }

        let ranking_refs: Vec<&[(String, f64)]> = rankings.iter().map(|r| r.as_slice()).collect();
        let fused =
            crate::retrieval_fusion::rrf_fuse(&ranking_refs, crate::retrieval_fusion::RRF_K);
        let mut out = Vec::with_capacity(fused.len());
        for (id, score) in fused {
            if let Some(mut item) = by_id.remove(&id) {
                item.score = score;
                out.push(item);
            }
        }
        // Keep any leftovers that RRF dropped (stable append by prior score).
        let mut rest: Vec<_> = by_id.into_values().collect();
        rest.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.extend(rest);
        Ok(out)
    }

    /// Record that a file was accessed by an MCP tool, for auto-capture at next route_task.
    pub fn record_file_access(&self, tool: &str, path: &str) {
        if !self.config.auto_capture_enabled {
            return;
        }
        if let Ok(mut f) = self.touched_files.lock() {
            let p = path.to_string();
            if !f.contains(&p) {
                f.push(p);
            }
        }
        if let Ok(mut t) = self.prev_tool_names.lock() {
            let n = tool.to_string();
            if !t.contains(&n) {
                t.push(n);
            }
        }
    }

    /// Per-phase TTL for route cache. Debugging and verification are shorter-lived;
    /// architecture and review can cache longer since those artifacts change slowly.
    fn phase_ttl(&self, phase: &str) -> Duration {
        let base = Duration::from_secs(self.config.turn_ttl_secs);
        match phase {
            "debugging" => base.min(Duration::from_secs(30)),
            "verification" | "testing" => base.min(Duration::from_secs(45)),
            "reviewing" => base.max(Duration::from_secs(120)),
            "planning" | "architecture" => base.max(Duration::from_secs(180)),
            _ => base,
        }
    }

    pub fn route_task(
        &self,
        user_message: &str,
        cwd: Option<&Path>,
        open_files: &[String],
        max_tokens: usize,
        limits: RouteLimits,
        explicit_phase: Option<&str>,
        explicit_task_kind: Option<&str>,
    ) -> Result<RouteTaskResponse> {
        self.maybe_spawn_session_ingest();

        // Auto-store memory for the previous turn if MCP tools were used since last route_task.
        // This captures what actually happened between turns without explicit store_memory calls.
        if self.config.auto_capture_enabled && self.mcp_activity.tools_used_since_last_route() {
            if let Ok(prev) = self.prev_briefing.lock() {
                if let Some(ref briefing) = *prev {
                    if !briefing.is_empty() {
                        let files = self
                            .touched_files
                            .lock()
                            .ok()
                            .map(|f| f.clone())
                            .unwrap_or_default();
                        let tools = self
                            .prev_tool_names
                            .lock()
                            .ok()
                            .map(|t| t.clone())
                            .unwrap_or_default();
                        let mut extras = Vec::new();
                        if !tools.is_empty() {
                            extras.push(format!("tools: {}", tools.join(", ")));
                        }
                        if !files.is_empty() {
                            extras.push(format!("files: {}", files.join(", ")));
                        }
                        let fact = if extras.is_empty() {
                            format!("Worked on: {briefing}")
                        } else {
                            format!("Worked on: {briefing} ({})", extras.join("; "))
                        };
                        let _ = self.write_queue.send(crate::db::WriteOp::StoreMemory {
                            resp_tx: std::sync::mpsc::channel().0,
                            payload:
                                crate::db::write_queue::store_memory_payload::StoreMemoryRequest {
                                    topic: "auto-route".into(),
                                    fact,
                                    scope: "project".into(),
                                    scope_key: None,
                                    confidence: 0.5,
                                    polarity: None,
                                    apply_when: None,
                                    valid_from: None,
                                    invalid_at: None,
                                },
                        });
                    }
                }
            }
            // Clear turn-local buffers after auto-store
            if let Ok(mut f) = self.touched_files.lock() {
                f.clear();
            }
            if let Ok(mut t) = self.prev_tool_names.lock() {
                t.clear();
            }
        }

        let task_kind = crate::bridge::resolve_task_kind(explicit_task_kind, user_message);
        let limits = crate::bridge::limits_for_task_kind(task_kind, limits);
        let limits = limits.normalize();
        let started = Instant::now();
        let ws = probe(cwd);
        let phase = explicit_phase
            .filter(|p| !p.is_empty() && *p != "unknown")
            .map(|p| p.to_string())
            .unwrap_or_else(|| infer_phase(user_message));
        let project_identity = cwd.and_then(|path| {
            let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            crate::project_key::resolve_project_key(&resolved).ok()
        });
        if let Some(ref id) = project_identity {
            let domain = self
                .store
                .domain_for_project_key(&id.project_key)
                .unwrap_or_else(|_| "unclassified".to_string());
            let _ = self
                .store
                .upsert_project_root(&id.project_key, &id.repo_root, &domain);
        }
        // Prefer path-independent project_key for cache + project scope matching.
        let scope_key = project_identity
            .as_ref()
            .map(|id| id.project_key.clone())
            .or_else(|| ws.repo_root.clone())
            .unwrap_or_default();
        let project_key_opt = project_identity.as_ref().map(|id| id.project_key.as_str());
        let cache_warm = self.warmed.load(Ordering::Relaxed);

        let cache_key = route_cache_key(
            &scope_key,
            &phase,
            task_kind.as_str(),
            open_files,
            user_message,
            self.store.get_index_version(),
            self.config.turn_cache_ignore_open_files,
        );

        let phase_ttl = self.phase_ttl(&phase);
        if let Some(mut cached) = self.cache.get_with_ttl(&cache_key, phase_ttl) {
            if is_empty_route_response(&cached) {
                self.cache.remove(&cache_key);
            } else {
                if let Some(repo) = ws.repo_root.as_deref() {
                    cached.repo_snapshot = crate::repo_snapshot::capture(
                        std::path::Path::new(repo),
                        &self.config.home,
                    );
                }
                let total_us = started.elapsed().as_micros() as u64;
                cached.latency_ms = total_us / 1000;
                let timing = RouteTiming {
                    total_us,
                    cache_hit: true,
                    cache_warm,
                    ..Default::default()
                };
                let p95 = self.route_latency.p95_ms();
                self.route_latency.record(timing.total_us / 1000);
                timing.log_line(p95, &phase);
                return Ok(self.finish_route_response(cached));
            }
        }

        if self.config.session_stickiness_secs > 0 {
            let session_key = session_route_cache_key(
                &scope_key,
                &phase,
                task_kind.as_str(),
                open_files,
                self.store.get_index_version(),
                self.config.turn_cache_ignore_open_files,
            );
            let session_ttl = Duration::from_secs(self.config.session_stickiness_secs);
            if let Some(mut cached) = self.cache.get_with_ttl(&session_key, session_ttl) {
                if let Some(repo) = ws.repo_root.as_deref() {
                    cached.repo_snapshot = crate::repo_snapshot::capture(
                        std::path::Path::new(repo),
                        &self.config.home,
                    );
                }
                let total_us = started.elapsed().as_micros() as u64;
                cached.latency_ms = total_us / 1000;
                return Ok(self.finish_route_response(cached));
            }
        }

        let scratchpad_keywords: Vec<String> = if let Some(repo) = ws.repo_root.as_deref() {
            if let Ok(entries) = self.store.read_scratchpad(repo, 10, None) {
                let mut kws: Vec<String> = Vec::new();
                for e in &entries {
                    for word in e.content.split_whitespace() {
                        let w = word.trim_matches(|c: char| !c.is_alphanumeric());
                        if w.len() >= 4 {
                            kws.push(w.to_lowercase());
                        }
                    }
                }
                kws.sort();
                kws.dedup();
                kws.truncate(12);
                kws
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let mut all_tags = ws.tags.clone();
        all_tags.extend(scratchpad_keywords);
        let mcp_tags = mcp_route_expansion_tags(user_message);
        all_tags.extend(mcp_tags.clone());

        let query = if mcp_tags.is_empty() {
            format!("{} ", user_message.trim_end())
        } else {
            format!("{} {}", user_message, mcp_tags.join(" "))
        };
        let message_fp = fingerprint_query(user_message);
        let (scored, candidates, index_total, embed_us, score_us, embed_cache_hit, bm25_fast_path) =
            self.route_query_parallel(
                &query,
                &message_fp,
                // Prefer project_key for scope matching; fall back to path for legacy facts.
                project_key_opt.or(ws.repo_root.as_deref()),
                &all_tags,
                agent_boost_keywords(user_message),
                &phase,
                open_files,
                user_message,
            )?;

        let build_started = Instant::now();
        let mut resp = build_route_response(
            &scored,
            &limits,
            &phase,
            max_tokens,
            project_key_opt,
            ws.repo_root.as_deref(),
            &ws.tags,
            user_message,
        );
        resp.index_total = index_total;
        if let Some(repo) = ws.repo_root.as_deref() {
            resp.repo_snapshot =
                crate::repo_snapshot::capture(std::path::Path::new(repo), &self.config.home);
            if let Ok(entries) = self.store.read_scratchpad(repo, 10, None) {
                resp.scratchpad = entries;
            }
        }
        let settings = crate::settings::AgentBrainSettings::load(&self.config.home);
        resp.suggested_tools = crate::upstream::suggest_upstream_tools(
            &self.store,
            &settings.upstream_mcp,
            user_message,
            settings.upstream_mcp.suggest_limit,
        );
        resp.must_apply
            .push(crate::capabilities::capability_rule_for(
                &self.store,
                &settings.upstream_mcp,
            ));
        resp.suggested_native_tools = crate::token_tools::suggest_native_token_tools(
            user_message,
            open_files,
            &phase,
            &resp.must_apply,
        );
        resp.verified_recipes =
            crate::verify::suggest_verified_recipes(&self.store, user_message, 3);
        if let Some(top) = resp.verified_recipes.first() {
            let cmd_display = if top.cmd.len() > 200 {
                format!("{}...", &top.cmd[..197])
            } else {
                top.cmd.clone()
            };
            resp.must_apply.push(crate::types::MustApply {
                topic: "verified-recipe".into(),
                text: format!(
                    "Prefer verified recipe `{}` (exit 0 verified via {}) over unverified raw commands.",
                    cmd_display, top.source
                ),
            });
        }
        resp.must_apply.push(crate::types::MustApply {
            topic: "agstack-invariants".into(),
            text: "Mandatory agstack invariants: 1. Immutability first (never mutate in-place; return fresh copies). 2. Zero AI comment slop (self-documenting code; pre-commit rejects comment lines). 3. Boundary discipline (validate schemas at boundaries). 4. Fix root causes (never mask with defensive null-checks; verify with regression test).".into(),
        });
        #[cfg(feature = "extras")]
        if let Some(repo) = ws.repo_root.as_deref() {
            if settings.graphify.enabled {
                resp.code_context = crate::graphify::route_code_context(
                    &self.store,
                    std::path::Path::new(repo),
                    user_message,
                    100,
                );
            }
        }
        let topics: Vec<String> = resp
            .relevant_memory
            .iter()
            .map(|m| m.topic.clone())
            .collect();
        for (topic, message) in self.store.scope_conflict_warnings(&topics)? {
            resp.warnings
                .push(crate::types::RouteWarning { topic, message });
        }
        let build_us = build_started.elapsed().as_micros() as u64;

        let total_us = started.elapsed().as_micros() as u64;
        resp.cache_hit = false;
        resp.latency_ms = total_us / 1000;
        resp.log_id = Uuid::new_v4().to_string();

        let timing = RouteTiming {
            embed_us,
            score_us,
            build_us,
            total_us,
            cache_hit: false,
            embed_cache_hit,
            cache_warm,
            bm25_fast_path,
            candidates,
            index_total,
        };
        let p95 = self.route_latency.p95_ms();
        self.route_latency.record(timing.total_us / 1000);
        timing.log_line(p95, &phase);

        crate::bridge::enrich_route_response(&mut resp, &scored, task_kind, user_message);

        let memory_ids: Vec<String> = scored
            .iter()
            .filter(|s| matches!(s.item_type, crate::types::ItemType::Memory))
            .map(|s| s.id.clone())
            .collect();
        if !memory_ids.is_empty() {
            if let Ok(stats) = self.store.load_retrieval_stats(&memory_ids) {
                let mut retrieval_stats = Vec::new();
                for scored_item in &scored {
                    if !matches!(scored_item.item_type, crate::types::ItemType::Memory) {
                        continue;
                    }
                    if let Some(&(useful, useless)) = stats.get(&scored_item.id) {
                        if useful > 0 || useless > 0 {
                            retrieval_stats.push(crate::types::MemoryRetrievalStat {
                                topic: scored_item.topic.clone(),
                                useful_count: useful,
                                useless_count: useless,
                            });
                        }
                    }
                }
                resp.retrieval_stats = retrieval_stats;
            }
        }

        let memory_topics: Vec<String> = resp
            .relevant_memory
            .iter()
            .map(|m| m.topic.clone())
            .collect();
        if !memory_topics.is_empty() {
            let store = self.store.clone();
            let embedder = self.embedder.clone();
            let _config = self.config.clone();
            std::thread::spawn(move || {
                if let Err(err) = auto_observe_topics(&store, &embedder, &memory_topics) {
                    tracing::warn!(error = %err, "auto-observe failed");
                }
            });
        }

        if !is_empty_route_response(&resp) {
            self.cache.put(cache_key, resp.clone());
            if self.config.session_stickiness_secs > 0 {
                let session_key = session_route_cache_key(
                    &scope_key,
                    &phase,
                    task_kind.as_str(),
                    open_files,
                    self.store.get_index_version(),
                    self.config.turn_cache_ignore_open_files,
                );
                self.cache.put(session_key, resp.clone());
            }
        }

        if let Err(err) = crate::observability::log_route(
            &self.store,
            &resp.log_id,
            user_message,
            &phase,
            &resp,
            &scored,
            false,
        ) {
            tracing::warn!(error = %err, "retrieval log write failed");
        }

        let resp = self.finish_route_response(resp);
        let briefing = resp.briefing.clone();
        if !briefing.is_empty() {
            if let Ok(mut prev) = self.prev_briefing.lock() {
                *prev = Some(briefing);
            }
        }
        self.mcp_activity.record_route();
        Ok(resp)
    }

    fn finish_route_response(&self, mut resp: RouteTaskResponse) -> RouteTaskResponse {
        if resp.index_total == 0 {
            if let Ok(n) = self.store.count_indexed_items() {
                resp.index_total = n;
            }
        }
        if self.config.mcp_gate_enabled {
            resp.warnings.push(crate::types::RouteWarning {
                topic: "mcp_contract".into(),
                message: "Session digests (Cursor/OpenCode/Codex/Gemini/Antigravity) and team memory surface only through route_task. Other agent-brain MCP tools are gated until route_task succeeds each turn.".into(),
            });
            resp.warnings.push(crate::types::RouteWarning {
                topic: "native_tools".into(),
                message: "Prefer agent-brain grep_search, file_summary, read_file_head, read_file_tail over host Read/Grep/Cat — host native tools bypass routing and cross-agent ingest.".into(),
            });
        }
        if self.config.route_briefing_enabled {
            route_briefing::publish_briefing(
                &self.config.home,
                &self.config.logs_dir,
                &resp,
                self.config.route_briefing_stderr,
                Some(&self.store),
            );
            resp.briefing = route_briefing::format_summary_line(&resp);

            // Session-aware delta: compare against previous turn's response
            if let Ok(mut prev) = self.prev_snapshot.lock() {
                let cur_snapshot = ResponseSnapshot::from(&resp);
                let delta = compute_delta_from_snapshot(prev.as_ref(), &cur_snapshot);
                if !delta.is_empty() {
                    if resp.briefing.is_empty() {
                        resp.briefing = delta;
                    } else {
                        resp.briefing.push_str(" | ");
                        resp.briefing.push_str(&delta);
                    }
                }
                *prev = Some(cur_snapshot);
            }
        }

        resp
    }
}

#[derive(Debug, Clone)]
struct ResponseSnapshot {
    agent_count: usize,
    phase: String,
    task_kind: Option<String>,
    cache_hit: bool,
    confidence: f64,
    escalate: bool,
    latency_ms: u64,
}

impl From<&RouteTaskResponse> for ResponseSnapshot {
    fn from(r: &RouteTaskResponse) -> Self {
        Self {
            agent_count: r.recommended_agents.len(),
            phase: r.recommended_phase.clone(),
            task_kind: r.task_kind.clone(),
            cache_hit: r.cache_hit,
            confidence: r.route_confidence,
            escalate: r.escalate_recommended,
            latency_ms: r.latency_ms,
        }
    }
}

fn compute_delta_from_snapshot(prev: Option<&ResponseSnapshot>, cur: &ResponseSnapshot) -> String {
    let Some(prev) = prev else {
        return String::new();
    };

    let mut parts: Vec<String> = Vec::new();

    let agents_diff = cur.agent_count as isize - prev.agent_count as isize;
    if agents_diff != 0 {
        let sign = if agents_diff > 0 { "+" } else { "" };
        parts.push(format!(
            "{}agents: {}{}",
            if agents_diff > 0 { "△ " } else { "" },
            sign,
            agents_diff
        ));
    }

    if prev.phase != cur.phase {
        parts.push(format!("phase: {}→{}", prev.phase, cur.phase));
    }

    if prev.task_kind != cur.task_kind {
        match (&prev.task_kind, &cur.task_kind) {
            (Some(p), Some(c)) => parts.push(format!("kind: {}→{}", p, c)),
            (None, Some(c)) => parts.push(format!("kind: →{}", c)),
            (Some(p), None) => parts.push(format!("kind: {}→", p)),
            _ => {}
        }
    }

    if prev.cache_hit != cur.cache_hit && cur.cache_hit {
        parts.push("cache".into());
    }

    if (cur.confidence - prev.confidence).abs() > 0.05 {
        parts.push(format!(
            "conf: {:.0}%→{:.0}%",
            prev.confidence * 100.0,
            cur.confidence * 100.0
        ));
    }

    if cur.escalate && !prev.escalate {
        parts.push("escalate".into());
    }

    let latency_change = cur.latency_ms as isize - prev.latency_ms as isize;
    if latency_change.abs() > 50 {
        parts.push(format!("{:+.0}ms", latency_change as f64));
    }

    if parts.is_empty() {
        String::new()
    } else {
        parts.join(", ")
    }
}

impl Engine {
    pub fn get_context(
        &self,
        task_description: &str,
        cwd: Option<&Path>,
        max_tokens: usize,
        include_types: &[ItemType],
    ) -> Result<GetContextResponse> {
        let ws = probe(cwd);
        let query = task_description.to_string();
        let message_fp = fingerprint_query(task_description);
        let (scored, _, _, _, _, _, _) = self.route_query_parallel(
            &query,
            &message_fp,
            ws.repo_root.as_deref(),
            &ws.tags,
            false,
            &infer_phase(task_description),
            &[],
            task_description,
        )?;

        let mut items: Vec<GetContextItem> = Vec::new();
        let mut tokens_used = 0;
        let mut truncated = false;
        let mut seen_topics = HashSet::new();

        let top_score = scored.first().map(|i| i.score).unwrap_or(0.0);
        let min_score = crate::retrieval::minimum_recommendation_score(top_score);

        for item in scored {
            if !include_types.contains(&item.item_type) {
                continue;
            }
            if item.score < min_score {
                continue;
            }
            if is_domain_incompatible(
                &item.topic,
                &item.text,
                item.source_path.as_deref().unwrap_or(""),
                &ws.tags,
                task_description,
            ) {
                continue;
            }
            if is_duplicate_topic(&mut seen_topics, &item) {
                continue;
            }
            if matches!(item.item_type, ItemType::Skill | ItemType::Agent) {
                let msg_lexical = crate::retrieval::lexical_overlap_score(
                    task_description,
                    &item.topic,
                    &item.text,
                );
                if msg_lexical == 0.0 && item.score < 0.65 {
                    continue;
                }
            }
            if let Some(top) = items.first() {
                let min_relative = (top.score * 0.70).max(top.score - 0.20);
                if item.score < min_relative {
                    continue;
                }
            }
            let entry = GetContextItem {
                item_type: item.item_type.as_str().to_string(),
                topic: item.topic.clone(),
                text: item.text.clone(),
                score: item.score,
                scope: item.scope.clone(),
                source_path: item.source_path.clone(),
            };
            let cost = estimate_json_tokens(&serde_json::to_value(&entry)?);
            if tokens_used + cost > max_tokens {
                truncated = true;
                break;
            }
            tokens_used += cost;
            items.push(entry);
        }

        Ok(GetContextResponse {
            items,
            tokens_used,
            tokens_budget: max_tokens,
            truncated,
        })
    }
}

fn is_duplicate_topic(seen: &mut HashSet<String>, item: &ScoredItem) -> bool {
    match item.item_type {
        ItemType::Agent | ItemType::Skill => !seen.insert(item.topic.to_ascii_lowercase()),
        _ => false,
    }
}

fn is_domain_incompatible(
    topic: &str,
    text: &str,
    path: &str,
    ws_tags: &[String],
    user_message: &str,
) -> bool {
    let msg_lower = user_message.to_lowercase();
    let target = format!(
        "{} {} {}",
        topic.to_lowercase(),
        text.to_lowercase(),
        path.to_lowercase()
    );

    let is_rust = ws_tags.iter().any(|t| t == "rust" || t == "cargo");
    let is_python = ws_tags.iter().any(|t| t == "python");
    let has_web = ws_tags.iter().any(|t| {
        t == "node"
            || t == "pnpm"
            || t == "yarn"
            || t == "npm"
            || t == "typescript"
            || t == "javascript"
            || t == "web"
    });

    if (is_rust || is_python) && !has_web {
        let web_markers = [
            "react",
            "nextjs",
            "next.js",
            "vue",
            "svelte",
            "tailwind",
            "css",
            "html",
            "browser-dom",
            "angular",
            "frontend",
            "jsx",
            "tsx",
            "vercel",
        ];
        let has_web_marker = web_markers.iter().any(|m| target.contains(m));
        if has_web_marker {
            let msg_has_web_intent = web_markers.iter().any(|m| msg_lower.contains(m));
            if !msg_has_web_intent {
                return true;
            }
        }
    }

    let is_js = ws_tags
        .iter()
        .any(|t| t == "typescript" || t == "javascript" || t == "node" || t == "pnpm");
    if is_js && !is_rust {
        let rust_markers = [
            "cargo", "clippy", "rust", "tokio", "diesel", "actix", "axum",
        ];
        let has_rust_marker = rust_markers.iter().any(|m| target.contains(m));
        if has_rust_marker {
            let msg_has_rust_intent = rust_markers.iter().any(|m| msg_lower.contains(m));
            if !msg_has_rust_intent {
                return true;
            }
        }
    }

    if is_js && !is_python {
        let py_markers = ["pytest", "django", "fastapi", "flask", "pydantic"];
        let has_py_marker = py_markers.iter().any(|m| target.contains(m));
        if has_py_marker {
            let msg_has_py_intent = py_markers.iter().any(|m| msg_lower.contains(m));
            if !msg_has_py_intent {
                return true;
            }
        }
    }

    false
}

fn build_route_response(
    scored: &[ScoredItem],
    limits: &RouteLimits,
    phase: &str,
    max_tokens: usize,
    project_key: Option<&str>,
    repo_root: Option<&str>,
    ws_tags: &[String],
    user_message: &str,
) -> RouteTaskResponse {
    let mut resp = RouteTaskResponse {
        recommended_phase: phase.to_string(),
        tokens_budget: max_tokens,
        ..Default::default()
    };
    let mut state = RouteBuildState::default();

    let top_non_memory = scored
        .iter()
        .filter(|i| !matches!(i.item_type, ItemType::Memory))
        .map(|i| i.score)
        .fold(0.0_f64, f64::max);
    let min_score = crate::retrieval::minimum_recommendation_score(top_non_memory);

    let calc_floor = |item_type: ItemType| -> f64 {
        let top = scored
            .iter()
            .filter(|i| i.item_type == item_type)
            .map(|i| i.score)
            .fold(0.0_f64, f64::max);
        if item_type == ItemType::Skill {
            if top >= 0.70 {
                min_score.max(0.48)
            } else if top >= 0.55 {
                min_score.max(0.42)
            } else {
                min_score.max(0.38)
            }
        } else if top >= 0.60 {
            min_score.max(0.42)
        } else if top >= 0.45 {
            min_score.max(0.38)
        } else {
            min_score.max(0.35)
        }
    };

    let skill_floor = calc_floor(ItemType::Skill);
    let agent_floor = calc_floor(ItemType::Agent);
    let rule_floor = calc_floor(ItemType::Rule);
    let workflow_floor = calc_floor(ItemType::Workflow);
    let memory_floor = {
        let top = scored
            .iter()
            .filter(|i| matches!(i.item_type, ItemType::Memory))
            .map(|i| i.score)
            .fold(0.0_f64, f64::max);
        if top >= 0.60 {
            min_score.max(0.35)
        } else {
            min_score
        }
    };

    resp.must_apply =
        collect_must_apply_from_scored(scored, 3, project_key, repo_root, ws_tags, user_message);

    // Pass 1: workflows, agents, skills, rules — before memory consumes the token budget.
    for item in scored.iter() {
        if matches!(item.item_type, ItemType::Memory) {
            continue;
        }
        let type_floor = match item.item_type {
            ItemType::Skill => skill_floor,
            ItemType::Agent => agent_floor,
            ItemType::Rule => rule_floor,
            ItemType::Workflow => workflow_floor,
            _ => min_score,
        };
        if item.score < type_floor {
            continue;
        }
        if is_domain_incompatible(
            &item.topic,
            &item.text,
            item.source_path.as_deref().unwrap_or(""),
            ws_tags,
            user_message,
        ) {
            continue;
        }

        // Margin: additional items must stay close to the top hit of their type.
        match item.item_type {
            ItemType::Skill => {
                let is_agstack = item
                    .source_path
                    .as_deref()
                    .map_or(false, |p| p.contains("agstack"));

                if crate::retrieval::is_conversational_or_meta(user_message) && !is_agstack {
                    continue;
                }
                if crate::retrieval::query_forbids_topic(user_message, &item.topic) {
                    continue;
                }

                let msg_lexical =
                    crate::retrieval::lexical_overlap_score(user_message, &item.topic, &item.text);
                let topic_aff = crate::retrieval::skill_topic_affinity(user_message, &item.topic);
                let is_info = crate::retrieval::is_informational_query(user_message);

                if !is_agstack && !user_message.is_empty() {
                    let has_ws_affinity = ws_tags
                        .iter()
                        .any(|t| item.topic.to_lowercase().contains(t.to_lowercase().as_str()));
                    if is_info {
                        let action_match = topic_aff >= 0.5 && (topic_aff == 1.0 || item.score >= 0.68);
                        if !action_match && item.score < 0.85 {
                            continue;
                        }
                    } else if (topic_aff == 0.0 && msg_lexical < 0.40 && !has_ws_affinity && item.score < 0.80)
                        || (topic_aff < 0.5 && msg_lexical < 0.30 && !has_ws_affinity && item.score < 0.72)
                    {
                        continue;
                    }
                }

                if msg_lexical == 0.0 && item.score < 0.65 && !is_agstack {
                    continue;
                }
                if let Some(top) = resp.recommended_skills.first() {
                    let min_relative = (top.score * 0.70).max(top.score - 0.18);
                    if item.score < min_relative && !is_agstack {
                        continue;
                    }
                }
            }
            ItemType::Agent => {
                let msg_lexical =
                    crate::retrieval::lexical_overlap_score(user_message, &item.topic, &item.text);
                if msg_lexical == 0.0 && item.score < 0.65 {
                    continue;
                }
                if let Some(top) = resp.recommended_agents.first() {
                    let min_relative = (top.score * 0.70).max(top.score - 0.15);
                    if item.score < min_relative {
                        continue;
                    }
                }
            }
            ItemType::Rule => {
                if let Some(top) = resp.applicable_rules.first() {
                    let min_relative = (top.score * 0.70).max(top.score - 0.18);
                    if item.score < min_relative {
                        continue;
                    }
                }
            }
            ItemType::Workflow => {
                if resp.suggested_workflow.is_some() {
                    continue;
                }
            }
            _ => {}
        }

        try_add_route_item(
            &mut resp,
            &mut state,
            item,
            limits,
            phase,
            max_tokens,
            project_key,
            repo_root,
        );
    }

    if resp.recommended_skills.is_empty()
        && limits.skills > 0
        && !crate::retrieval::is_conversational_or_meta(user_message)
    {
        if let Some(agstack_item) = scored.iter().find(|i| {
            matches!(i.item_type, ItemType::Skill)
                && i.source_path
                    .as_deref()
                    .map_or(false, |p| p.contains("agstack"))
        }) {
            try_add_route_item(
                &mut resp,
                &mut state,
                agstack_item,
                limits,
                phase,
                max_tokens,
                project_key,
                repo_root,
            );
        }
    }

    // Pass 2: memory with whatever budget remains (same score floor; domain filter; no digest/obs noise).
    for item in scored.iter() {
        if !matches!(item.item_type, ItemType::Memory) {
            continue;
        }
        if item.score < memory_floor {
            continue;
        }
        if is_domain_incompatible(
            &item.topic,
            &item.text,
            item.source_path.as_deref().unwrap_or(""),
            ws_tags,
            user_message,
        ) {
            continue;
        }
        if let Some(top) = resp.relevant_memory.first() {
            let min_relative = (top.score * 0.65).max(top.score - 0.22);
            if item.score < min_relative {
                continue;
            }
        }
        try_add_route_item(
            &mut resp,
            &mut state,
            item,
            limits,
            phase,
            max_tokens,
            project_key,
            repo_root,
        );
    }

    resp
}

#[derive(Default)]
struct RouteBuildState {
    seen_agents: HashSet<String>,
    seen_skills: HashSet<String>,
}

fn try_add_route_item(
    resp: &mut RouteTaskResponse,
    state: &mut RouteBuildState,
    item: &ScoredItem,
    limits: &RouteLimits,
    phase: &str,
    max_tokens: usize,
    project_key: Option<&str>,
    repo_root: Option<&str>,
) {
    enum PendingRec {
        Agent(AgentRec),
        Skill(SkillRec),
        Rule(RuleRec),
        Memory(MemoryRec),
    }

    let (pending, rec_tokens) = match item.item_type {
        ItemType::Agent if resp.recommended_agents.len() < limits.agents => {
            if !state.seen_agents.insert(item.topic.to_ascii_lowercase()) {
                return;
            }
            let rec = AgentRec {
                name: item.topic.clone(),
                path: item.source_path.clone().unwrap_or_default(),
                rationale: rationale_for(item, phase),
                score: item.score,
            };
            let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
            (PendingRec::Agent(rec), t)
        }
        ItemType::Skill if resp.recommended_skills.len() < limits.skills => {
            if !state.seen_skills.insert(item.topic.to_ascii_lowercase()) {
                return;
            }
            let rec = SkillRec {
                name: item.topic.clone(),
                path: item.source_path.clone().unwrap_or_default(),
                rationale: rationale_for(item, phase),
                score: item.score,
                text: None,
            };
            let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
            (PendingRec::Skill(rec), t)
        }
        ItemType::Rule if resp.applicable_rules.len() < limits.rules => {
            let rec = RuleRec {
                topic: item.topic.clone(),
                text: item.text.chars().take(300).collect(),
                source_path: item.source_path.clone().unwrap_or_default(),
                score: item.score,
            };
            let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
            (PendingRec::Rule(rec), t)
        }
        ItemType::Memory if resp.relevant_memory.len() < limits.memory => {
            if is_low_signal_memory(&item.topic, None) {
                return;
            }
            let rec = MemoryRec {
                topic: item.topic.clone(),
                text: item.text.chars().take(300).collect(),
                score: item.score,
            };
            let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
            (PendingRec::Memory(rec), t)
        }
        ItemType::Workflow => {
            if resp.suggested_workflow.is_none() && item.score >= 0.35 {
                let workflow_slug = item.topic.clone();
                resp.suggested_workflow = Some(workflow_slug.clone());
                if resp.must_apply.len() < 3 {
                    let rec = MustApply {
                        topic: "spine-workflow".to_string(),
                        text: format!(
                            "Recommended Spine workflow: `{}` (relevance: {:.2}). Orchestrate via `spine_submit_workflow` (workflow=\"{}\") or `agent-spine run --workflow {}`.",
                            workflow_slug, item.score, workflow_slug, workflow_slug
                        ),
                    };
                    let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
                    if resp.tokens_used + t <= max_tokens {
                        resp.tokens_used += t;
                        resp.must_apply.push(rec);
                    }
                }
            }
            return;
        }
        _ => return,
    };

    if resp.tokens_used + rec_tokens > max_tokens {
        return;
    }

    resp.tokens_used += rec_tokens;
    match pending {
        PendingRec::Agent(rec) => resp.recommended_agents.push(rec),
        PendingRec::Skill(rec) => resp.recommended_skills.push(rec),
        PendingRec::Rule(rec) => resp.applicable_rules.push(rec),
        PendingRec::Memory(rec) => {
            if let Some(constraint) = must_apply_for_memory(item, project_key, repo_root) {
                if resp.must_apply.len() < 3
                    && !resp.must_apply.iter().any(|m| m.topic == constraint.topic)
                {
                    resp.must_apply.push(constraint);
                }
            }
            resp.relevant_memory.push(rec);
        }
    }
}

fn is_empty_route_response(resp: &RouteTaskResponse) -> bool {
    resp.recommended_agents.is_empty()
        && resp.recommended_skills.is_empty()
        && resp.applicable_rules.is_empty()
        && resp.relevant_memory.is_empty()
}

fn rationale_for(item: &ScoredItem, phase: &str) -> String {
    format!(
        "Matched {} for {} phase (score {:.2}).",
        item.topic, phase, item.score
    )
}

fn project_scope_matches(
    item: &ScoredItem,
    project_key: Option<&str>,
    repo_root: Option<&str>,
) -> bool {
    if item.scope != "project" {
        return false;
    }
    match item.scope_key.as_deref() {
        None => true,
        Some(key) => {
            if project_key == Some(key) {
                return true;
            }
            // Legacy absolute-path scope_key from pre-project_key stores.
            if repo_root == Some(key) {
                return true;
            }
            false
        }
    }
}

fn must_apply_for_memory(
    item: &ScoredItem,
    project_key: Option<&str>,
    repo_root: Option<&str>,
) -> Option<MustApply> {
    if !matches!(item.item_type, ItemType::Memory) {
        return None;
    }
    // Digests / observations / auto-route never bind as hard constraints.
    if is_low_signal_memory(&item.topic, None) {
        return None;
    }
    let labeled_negative = item.polarity.as_deref() == Some("negative");
    let is_trap = item.topic.starts_with("trap-") || item.topic.starts_with("anti-pattern");
    let heuristic_negative =
        item.text.to_lowercase().contains("do not") || item.text.to_lowercase().contains("never ");
    let scoped = project_scope_matches(item, project_key, repo_root);

    let promote = (item.apply_when_matched && item.score >= 0.55 && item.lexical >= 0.15)
        || (is_trap && (item.score >= 0.35 || item.lexical >= 0.15))
        || (labeled_negative
            && item.score >= 0.55
            && item.lexical >= 0.20
            && (scoped || item.apply_when_matched))
        || (heuristic_negative
            && !labeled_negative
            && item.score >= 0.85
            && item.lexical >= 0.25
            && item.apply_when_matched);

    if promote {
        let display_text = if is_trap {
            format!("TRAP WARNING: {}", item.text.chars().take(180).collect::<String>())
        } else {
            item.text.chars().take(200).collect()
        };
        Some(MustApply {
            topic: item.topic.clone(),
            text: display_text,
        })
    } else {
        None
    }
}

fn collect_must_apply_from_scored(
    scored: &[ScoredItem],
    max: usize,
    project_key: Option<&str>,
    repo_root: Option<&str>,
    ws_tags: &[String],
    user_message: &str,
) -> Vec<MustApply> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut memories: Vec<&ScoredItem> = scored
        .iter()
        .filter(|i| matches!(i.item_type, ItemType::Memory))
        .collect();
    // Prefer project-scoped constraints over global when scores are close.
    memories.sort_by(|a, b| {
        let a_proj = if a.scope == "project" { 1 } else { 0 };
        let b_proj = if b.scope == "project" { 1 } else { 0 };
        b_proj.cmp(&a_proj).then_with(|| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    for item in memories {
        if out.len() >= max {
            break;
        }
        if is_domain_incompatible(
            &item.topic,
            &item.text,
            item.source_path.as_deref().unwrap_or(""),
            ws_tags,
            user_message,
        ) {
            continue;
        }
        if let Some(constraint) = must_apply_for_memory(item, project_key, repo_root) {
            if seen.insert(constraint.topic.clone()) {
                out.push(constraint);
            }
        }
    }
    out
}

/// After `route_task`, check retrieved memory topics for recurrence and auto-synthesize observations.
fn auto_observe_topics(
    store: &crate::db::store::BrainStore,
    embedder: &crate::embed::Embedder,
    topics: &[String],
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let since_ms = now - 90 * 24 * 3600 * 1000;
    for topic in topics {
        if topic.starts_with("obs/") || topic.starts_with("session-digest-") {
            continue;
        }
        let obs_topic = crate::observation::observation_topic(topic);
        if store.fact_exists_by_topic(&obs_topic)? {
            continue;
        }
        let count = store.count_active_facts_for_topic(topic, since_ms)?;
        if count < 3 {
            continue;
        }
        let snippet: String = topic.chars().take(80).collect();
        let fact = format!("Recurring topic across {count} facts: {snippet}");
        let embedding = embedder.embed_one(&format!("{obs_topic} {fact}"))?;
        let hash = crate::db::store::content_hash(&fact);
        let _ = store.store_fact_full(
            &obs_topic,
            &fact,
            "project",
            None,
            0.85,
            "observation",
            &hash,
            &embedding,
            "positive",
            None,
            None,
        )?;
        tracing::info!(topic = %obs_topic, facts = count, "auto-observed recurring pattern");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ItemType, RouteLimits, ScoredItem};

    fn mem(topic: &str, score: f64, text: &str) -> ScoredItem {
        ScoredItem {
            id: topic.into(),
            item_type: ItemType::Memory,
            topic: topic.into(),
            text: text.into(),
            source_path: None,
            scope: "project".into(),
            score,
            polarity: None,
            apply_when_matched: false,
            lexical: 0.0,
            scope_key: None,
        }
    }

    fn skill(topic: &str, score: f64) -> ScoredItem {
        ScoredItem {
            id: topic.into(),
            item_type: ItemType::Skill,
            topic: topic.into(),
            text: "skill body".into(),
            source_path: Some("/tmp/SKILL.md".into()),
            scope: "project".into(),
            score,
            polarity: None,
            apply_when_matched: false,
            lexical: 0.0,
            scope_key: None,
        }
    }

    #[test]
    fn route_excludes_digest_and_obs_from_memory_and_must_apply() {
        let scored = vec![
            mem(
                "session-digest-cursor-abc",
                0.99,
                "Retrieved only via route_task. Session digest ...",
            ),
            mem("obs/auto-route", 0.95, "Recurring pattern on auto-route"),
            mem("auto-route", 0.9, "Worked on: phase=explore"),
            mem(
                "project-convention",
                0.5,
                "Use agent-brain route_task every turn",
            ),
            skill("mcp-server-patterns", 0.8),
        ];
        let limits = RouteLimits {
            agents: 2,
            skills: 3,
            rules: 3,
            memory: 5,
        };
        let resp = build_route_response(&scored, &limits, "implementing", 800, None, None, &[], "");
        assert!(
            resp.relevant_memory.iter().all(|m| {
                !m.topic.starts_with("session-digest-")
                    && !m.topic.starts_with("obs/")
                    && m.topic != "auto-route"
            }),
            "noise leaked into relevant_memory: {:?}",
            resp.relevant_memory
        );
        assert!(
            resp.must_apply.iter().all(|m| {
                !m.topic.starts_with("session-digest-")
                    && !m.topic.starts_with("obs/")
                    && m.topic != "auto-route"
            }),
            "noise leaked into must_apply: {:?}",
            resp.must_apply
        );
        assert!(
            resp.relevant_memory
                .iter()
                .any(|m| m.topic == "project-convention"),
            "real memory should remain"
        );
    }

    #[test]
    fn must_apply_requires_score_even_with_apply_when() {
        let weak = ScoredItem {
            id: "cinema".into(),
            item_type: ItemType::Memory,
            topic: "slip-district-seat-fix".into(),
            text: "never use SCREEN N as seat".into(),
            source_path: None,
            scope: "project".into(),
            score: 0.25,
            polarity: None,
            apply_when_matched: true,
            lexical: 0.0,
            scope_key: None,
        };
        assert!(must_apply_for_memory(&weak, None, None).is_none());
        let strong = ScoredItem {
            score: 0.55,
            lexical: 0.20,
            ..weak
        };
        assert!(must_apply_for_memory(&strong, None, None).is_some());

        // Cross-project cinema slip must not bind on an unrelated architecture query.
        let cinema = ScoredItem {
            id: "cinema2".into(),
            item_type: ItemType::Memory,
            topic: "slip-district-seat-fix".into(),
            text: "never use SCREEN N as seat".into(),
            source_path: None,
            scope: "global".into(),
            score: 0.95,
            polarity: Some("negative".into()),
            apply_when_matched: false,
            lexical: 0.05,
            scope_key: None,
        };
        assert!(must_apply_for_memory(&cinema, None, Some("/tmp/agent-brain")).is_none());
    }

    #[test]
    fn trap_memory_promoted_with_warning_prefix() {
        let trap = ScoredItem {
            id: "trap-1".into(),
            item_type: ItemType::Memory,
            topic: "trap-inline-comments".into(),
            text: "do not add explanatory comment lines in rust files".into(),
            source_path: None,
            scope: "global".into(),
            score: 0.45,
            polarity: Some("negative".into()),
            apply_when_matched: false,
            lexical: 0.20,
            scope_key: None,
        };
        let res = must_apply_for_memory(&trap, None, None);
        assert!(res.is_some());
        let constraint = res.unwrap();
        assert!(constraint.text.starts_with("TRAP WARNING:"));
        assert_eq!(constraint.topic, "trap-inline-comments");
    }

    #[test]
    fn skill_margin_keeps_only_close_hits() {
        let scored = vec![
            skill("rust-patterns", 0.90),
            skill("mcp-server-patterns", 0.50),
            skill("make-interfaces-feel-better", 0.40),
        ];
        let limits = RouteLimits {
            agents: 0,
            skills: 3,
            rules: 0,
            memory: 0,
        };
        let resp = build_route_response(&scored, &limits, "planning", 800, None, None, &[], "");
        assert_eq!(resp.recommended_skills.len(), 1);
        assert_eq!(resp.recommended_skills[0].name, "rust-patterns");
    }

    #[test]
    fn skill_domain_incompatibility_filter() {
        let scored = vec![
            skill("vercel-react-patterns", 0.85),
            skill("rust-testing-guide", 0.75),
        ];
        let limits = RouteLimits {
            agents: 0,
            skills: 3,
            rules: 0,
            memory: 0,
        };
        let ws_tags = vec!["rust".to_string(), "cargo".to_string()];
        // 1. Rust workspace without web, user prompt does not mention react -> react skill filtered out, rust skill kept
        let resp = build_route_response(
            &scored,
            &limits,
            "implementing",
            800,
            None,
            None,
            &ws_tags,
            "implement tests for engine",
        );
        assert_eq!(resp.recommended_skills.len(), 1);
        assert_eq!(resp.recommended_skills[0].name, "rust-testing-guide");

        // 2. User prompt explicitly mentions react -> react skill allowed
        let resp_explicit = build_route_response(
            &scored,
            &limits,
            "implementing",
            800,
            None,
            None,
            &ws_tags,
            "refactor react frontend",
        );
        assert_eq!(resp_explicit.recommended_skills.len(), 2);
        assert_eq!(
            resp_explicit.recommended_skills[0].name,
            "vercel-react-patterns"
        );
    }

    #[test]
    fn skill_relative_dropoff_gap_drops_weak_secondary_skills() {
        let scored = vec![
            skill("rust-core-patterns", 0.90),
            skill("rust-advanced-unsafe", 0.78),
            skill("rust-random-utility", 0.55),
        ];
        let limits = RouteLimits {
            agents: 0,
            skills: 5,
            rules: 0,
            memory: 0,
        };
        let resp = build_route_response(
            &scored,
            &limits,
            "implementing",
            800,
            None,
            None,
            &["rust".to_string()],
            "refactor rust code",
        );
        // 0.90 top hit -> min_relative is max(0.90 * 0.70 = 0.63, 0.90 - 0.18 = 0.72) = 0.72.
        // 0.78 >= 0.72 -> kept
        // 0.55 < 0.72 -> dropped despite available limits
        assert_eq!(resp.recommended_skills.len(), 2);
        assert_eq!(resp.recommended_skills[0].name, "rust-core-patterns");
        assert_eq!(resp.recommended_skills[1].name, "rust-advanced-unsafe");
    }

    fn workflow(topic: &str, score: f64) -> ScoredItem {
        ScoredItem {
            id: format!("wf-{}", topic),
            item_type: ItemType::Workflow,
            topic: topic.into(),
            text: format!("Spine workflow for {}", topic),
            source_path: Some(format!("/tmp/workflows/{}.yaml", topic)),
            scope: "global".into(),
            score,
            polarity: None,
            apply_when_matched: false,
            lexical: 0.0,
            scope_key: None,
        }
    }

    fn agent(topic: &str, score: f64) -> ScoredItem {
        ScoredItem {
            id: format!("agent-{}", topic),
            item_type: ItemType::Agent,
            topic: topic.into(),
            text: format!("Specialist agent for {}", topic),
            source_path: Some(format!("/tmp/agents/{}.md", topic)),
            scope: "global".into(),
            score,
            polarity: None,
            apply_when_matched: false,
            lexical: 0.0,
            scope_key: None,
        }
    }

    fn rule(topic: &str, score: f64, text: &str) -> ScoredItem {
        ScoredItem {
            id: format!("rule-{}", topic),
            item_type: ItemType::Rule,
            topic: topic.into(),
            text: text.into(),
            source_path: Some(format!("/tmp/rules/{}.md", topic)),
            scope: "global".into(),
            score,
            polarity: None,
            apply_when_matched: false,
            lexical: 0.0,
            scope_key: None,
        }
    }

    #[test]
    fn workflow_domain_and_gap_filtering() {
        let scored = vec![
            workflow("react-component-pipeline", 0.88),
            workflow("engineering-safety", 0.75),
        ];
        let limits = RouteLimits {
            agents: 0,
            skills: 0,
            rules: 0,
            memory: 0,
        };
        let ws_tags = vec!["rust".to_string(), "cargo".to_string()];
        // 1. Rust workspace, non-web query: react workflow is domain-incompatible and filtered out, engineering-safety selected
        let resp = build_route_response(
            &scored,
            &limits,
            "implementing",
            800,
            None,
            None,
            &ws_tags,
            "verify safety gates before release",
        );
        assert_eq!(
            resp.suggested_workflow.as_deref(),
            Some("engineering-safety")
        );

        // 2. Explicit react prompt allows react workflow
        let resp_explicit = build_route_response(
            &scored,
            &limits,
            "implementing",
            800,
            None,
            None,
            &ws_tags,
            "run react component pipeline",
        );
        assert_eq!(
            resp_explicit.suggested_workflow.as_deref(),
            Some("react-component-pipeline")
        );
    }

    #[test]
    fn agent_and_rule_domain_and_gap_filtering() {
        let scored = vec![
            agent("rust-architect", 0.90),
            agent("react-frontend-dev", 0.82),
            agent("rust-junior-assistant", 0.55),
            rule("rust-safety-rule", 0.88, "Always use RAII and avoid unwrap"),
            rule("react-hooks-rule", 0.84, "Always follow React hooks rules"),
            rule("rust-minor-style", 0.50, "Prefer snake_case identifiers"),
        ];
        let limits = RouteLimits {
            agents: 5,
            skills: 0,
            rules: 5,
            memory: 0,
        };
        let ws_tags = vec!["rust".to_string(), "cargo".to_string()];
        let resp = build_route_response(
            &scored,
            &limits,
            "implementing",
            800,
            None,
            None,
            &ws_tags,
            "design rust architecture",
        );
        // Agents: react-frontend-dev filtered by domain; rust-junior-assistant (0.55) dropped by relative gap
        assert_eq!(resp.recommended_agents.len(), 1);
        assert_eq!(resp.recommended_agents[0].name, "rust-architect");

        // Rules: react-hooks-rule filtered by domain; rust-minor-style (0.50) dropped by relative gap
        assert_eq!(resp.applicable_rules.len(), 1);
        assert_eq!(resp.applicable_rules[0].topic, "rust-safety-rule");
    }
}
