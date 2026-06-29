use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::cache::{fingerprint_query, route_cache_key, session_route_cache_key, QueryEmbeddingCache, TurnCache};
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
use crate::workspace::{agent_boost_keywords, infer_phase, is_low_signal_memory, mcp_route_expansion_tags, probe};

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
        let mut n = index::sync_index_opts(&self.store, &self.config, &self.embedder, cwd, false, true)?;
        if self.config.session_ingest_enabled && !self.config.session_ingest_background {
            let sessions =
                crate::sessions::ingest_sessions(&self.store, &self.embedder, &self.config)?;
            if sessions > 0 {
                tracing::info!("session ingest: imported {sessions} session facts");
                self.store.bump_index_version()?;
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
        if sessions > 0 {
            self.store.bump_index_version()?;
        }
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
                    store.bump_index_version().ok();
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
        let indexed = index::sync_index_opts(&self.store, &self.config, &self.embedder, None, false, true)?;
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
        let (scored, candidates, index_total) = self.store.score_items_with_bm25(
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
            crate::ann::AnnSettings {
                enabled: self.config.ann_enabled,
                min_index: self.config.ann_min_index,
                top_k: self.config.ann_top_k,
            },
        )?;
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
                        let files = self.touched_files.lock().ok().map(|f| f.clone()).unwrap_or_default();
                        let tools = self.prev_tool_names.lock().ok().map(|t| t.clone()).unwrap_or_default();
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
                            payload: crate::db::write_queue::store_memory_payload::StoreMemoryRequest {
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
        let scope_key = ws.repo_root.clone().unwrap_or_default();
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
        all_tags.extend(mcp_route_expansion_tags(user_message));

        let query = format!("{} {}", user_message, all_tags.join(" "));
        let message_fp = fingerprint_query(user_message);
        let (scored, candidates, index_total, embed_us, score_us, embed_cache_hit, bm25_fast_path) =
            self.route_query_parallel(
                &query,
                &message_fp,
                ws.repo_root.as_deref(),
                &all_tags,
                agent_boost_keywords(user_message),
                &phase,
                open_files,
                user_message,
            )?;

        let build_started = Instant::now();
        let mut resp = build_route_response(&scored, &limits, &phase, max_tokens);
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
        resp.suggested_native_tools = crate::token_tools::suggest_native_token_tools(
            user_message,
            open_files,
            &phase,
            &resp.must_apply,
        );
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

        crate::bridge::enrich_route_response(&mut resp, &scored, task_kind);

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

        let memory_topics: Vec<String> = resp.relevant_memory.iter().map(|m| m.topic.clone()).collect();
        if !memory_topics.is_empty() {
            let store = self.store.clone();
            let embedder = self.embedder.clone();
            let config = self.config.clone();
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
        parts.push(format!("{}agents: {}{}", if agents_diff > 0 { "△ " } else { "" }, sign, agents_diff));
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
        parts.push(format!("conf: {:.0}%→{:.0}%", prev.confidence * 100.0, cur.confidence * 100.0));
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
        let query = format!("{} {}", task_description, ws.tags.join(" "));
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

        let mut items = Vec::new();
        let mut tokens_used = 0;
        let mut truncated = false;
        let mut seen_topics = HashSet::new();

        for item in scored {
            if !include_types.contains(&item.item_type) {
                continue;
            }
            if is_duplicate_topic(&mut seen_topics, &item) {
                continue;
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

fn build_route_response(
    scored: &[ScoredItem],
    limits: &RouteLimits,
    phase: &str,
    max_tokens: usize,
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

    resp.must_apply = collect_must_apply_from_scored(scored, 3);

    // Pass 1: agents, skills, rules — before memory consumes the token budget.
    for item in scored.iter() {
        if matches!(item.item_type, ItemType::Memory) {
            continue;
        }
        if item.score < min_score {
            continue;
        }
        try_add_route_item(&mut resp, &mut state, item, limits, phase, max_tokens);
    }

    // Pass 2: memory with whatever budget remains.
    for item in scored.iter() {
        if !matches!(item.item_type, ItemType::Memory) {
            continue;
        }
        try_add_route_item(&mut resp, &mut state, item, limits, phase, max_tokens);
    }

    resp
}

#[derive(Default)]
struct RouteBuildState {
    seen_agents: HashSet<String>,
    seen_skills: HashSet<String>,
    low_signal_memories: usize,
}

fn try_add_route_item(
    resp: &mut RouteTaskResponse,
    state: &mut RouteBuildState,
    item: &ScoredItem,
    limits: &RouteLimits,
    phase: &str,
    max_tokens: usize,
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
            if is_low_signal_memory(&item.topic, None) && state.low_signal_memories >= 1 {
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
            if resp.must_apply.len() < 3 {
                let rec = MustApply {
                    topic: "trigger_workflow".to_string(),
                    text: item.text.clone(),
                };
                let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
                if resp.tokens_used + t <= max_tokens {
                    resp.tokens_used += t;
                    resp.must_apply.push(rec);
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
            if is_low_signal_memory(&item.topic, None) {
                state.low_signal_memories += 1;
            }
            if let Some(constraint) = must_apply_for_memory(item) {
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

fn is_duplicate_topic(seen: &mut HashSet<String>, item: &ScoredItem) -> bool {
    match item.item_type {
        ItemType::Agent | ItemType::Skill => !seen.insert(item.topic.to_ascii_lowercase()),
        _ => false,
    }
}

fn must_apply_for_memory(item: &ScoredItem) -> Option<MustApply> {
    if !matches!(item.item_type, ItemType::Memory) {
        return None;
    }
    let is_negative = item.polarity.as_deref() == Some("negative")
        || item.text.to_lowercase().contains("do not")
        || item.text.to_lowercase().contains("never ");
    let negative_threshold = if item.polarity.as_deref() == Some("negative") {
        0.35
    } else {
        0.55
    };
    if (is_negative && item.score > negative_threshold) || item.apply_when_matched {
        Some(MustApply {
            topic: item.topic.clone(),
            text: item.text.chars().take(200).collect(),
        })
    } else {
        None
    }
}

fn collect_must_apply_from_scored(scored: &[ScoredItem], max: usize) -> Vec<MustApply> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut memories: Vec<&ScoredItem> = scored
        .iter()
        .filter(|i| matches!(i.item_type, ItemType::Memory))
        .collect();
    memories.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for item in memories {
        if out.len() >= max {
            break;
        }
        if let Some(constraint) = must_apply_for_memory(item) {
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
