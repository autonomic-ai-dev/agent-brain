use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::cache::{route_cache_key, fingerprint_query, QueryEmbeddingCache, TurnCache};
use crate::config::Config;
use crate::db::store::{content_hash, BrainStore};
use crate::db::{send_and_recv, spawn_write_handler, WriteOp, WriteQueue};
use crate::db::{RouteLatencyStats, RouteTiming};
use crate::embed::{parse_embedding_model, Embedder};
use crate::index;
use crate::tokens::estimate_json_tokens;
use crate::types::{
    AgentRec, GetContextItem, GetContextResponse, ItemType, MemoryRec, MustApply, RouteLimits,
    RouteTaskResponse, RuleRec, ScoredItem, SkillRec,
};
use crate::mcp_activity::McpActivity;
use crate::route_briefing;
use crate::sync::{ImportReport, MergePolicy, SyncSource};
use crate::workspace::{agent_boost_keywords, infer_phase, is_low_signal_memory, probe};

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
        let mut n = index::sync_index(&self.store, &self.config, &self.embedder, cwd)?;
        if self.config.session_ingest_enabled && !self.config.session_ingest_background {
            let sessions = crate::sessions::ingest_sessions(
                &self.store,
                &self.embedder,
                &self.config,
            )?;
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
            match crate::upstream::refresh_upstream_index_blocking(&settings.upstream_mcp, &self.store)
            {
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
            match crate::sessions::ingest_sessions(
                &engine.store,
                &engine.embedder,
                &engine.config,
            ) {
                Ok(sessions) if sessions > 0 => {
                    tracing::info!(
                        target: "agent_brain::sessions",
                        sessions,
                        "background session ingest complete"
                    );
                    engine.store.bump_index_version().ok();
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "background session ingest failed"),
            }
        });
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
                Ok(n) => tracing::info!(target: "agent_brain::bootstrap", items = n, "bootstrap complete"),
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

    pub fn route_task(
        &self,
        user_message: &str,
        cwd: Option<&Path>,
        open_files: &[String],
        max_tokens: usize,
        limits: RouteLimits,
        explicit_phase: Option<&str>,
    ) -> Result<RouteTaskResponse> {
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
            open_files,
            user_message,
            self.store.get_index_version(),
            self.config.turn_cache_ignore_open_files,
        );

        if let Some(mut cached) = self.cache.get(&cache_key) {
            if is_empty_route_response(&cached) {
                self.cache.remove(&cache_key);
            } else {
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

        let query = format!("{} {}", user_message, ws.tags.join(" "));
        let message_fp = fingerprint_query(user_message);
        let (scored, candidates, index_total, embed_us, score_us, embed_cache_hit, bm25_fast_path) =
            self.route_query_parallel(
                &query,
                &message_fp,
                ws.repo_root.as_deref(),
                &ws.tags,
                agent_boost_keywords(user_message),
                &phase,
                open_files,
                user_message,
            )?;

        let build_started = Instant::now();
        let mut resp = build_route_response(&scored, &limits, &phase, max_tokens);
        resp.index_total = index_total;
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
        let topics: Vec<String> = resp.relevant_memory.iter().map(|m| m.topic.clone()).collect();
        for (topic, message) in self.store.scope_conflict_warnings(&topics)? {
            resp.warnings.push(crate::types::RouteWarning { topic, message });
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

        if !is_empty_route_response(&resp) {
            self.cache.put(cache_key, resp.clone());
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

        Ok(self.finish_route_response(resp))
    }

    fn finish_route_response(&self, mut resp: RouteTaskResponse) -> RouteTaskResponse {
        if resp.index_total == 0 {
            if let Ok(n) = self.store.count_indexed_items() {
                resp.index_total = n;
            }
        }
        if self.config.route_briefing_enabled {
            route_briefing::publish_briefing(
                &self.config.home,
                &resp,
                self.config.route_briefing_stderr,
            );
        }
        resp.briefing = route_briefing::format_summary_line(&resp);
        resp
    }

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
                    && !resp
                        .must_apply
                        .iter()
                        .any(|m| m.topic == constraint.topic)
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
