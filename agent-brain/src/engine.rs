use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use uuid::Uuid;

use crate::cache::{fingerprint_open_files, fingerprint_query, CacheKey, TurnCache};
use crate::config::Config;
use crate::db::store::BrainStore;
use crate::embed::Embedder;
use crate::index;
use crate::tokens::estimate_json_tokens;
use crate::types::{
    AgentRec, GetContextItem, GetContextResponse, ItemType, MemoryRec, MustApply, RouteLimits,
    RouteTaskResponse, RuleRec, ScoredItem, SkillRec,
};
use crate::workspace::{agent_boost_keywords, infer_phase, probe};

pub struct Engine {
    pub config: Config,
    pub store: Arc<BrainStore>,
    pub embedder: Arc<Embedder>,
    pub cache: Arc<TurnCache>,
    pub auto_capture_enabled: bool,
}

impl Engine {
    pub fn new(config: Config) -> Result<Self> {
        config.ensure_dirs()?;
        let store = Arc::new(BrainStore::open(&config.db_path)?);
        let embedder = Arc::new(Embedder::new()?);
        let cache = Arc::new(TurnCache::new(64, config.turn_ttl_secs));
        Ok(Self {
            auto_capture_enabled: config.auto_capture_enabled,
            config,
            store,
            embedder,
            cache,
        })
    }

    pub fn bootstrap(&self, cwd: Option<&Path>) -> Result<usize> {
        let mut n = index::sync_index(&self.store, &self.config, &self.embedder, cwd)?;
        if self.config.session_ingest_enabled {
            let sessions = crate::sessions::ingest_legacy_sessions(
                &self.store,
                &self.embedder,
                &self.config,
            )?;
            if sessions > 0 {
                tracing::info!("session ingest: imported {sessions} legacy snippets");
            }
            n += sessions;
        }
        Ok(n)
    }

    pub fn route_task(
        &self,
        user_message: &str,
        cwd: Option<&Path>,
        open_files: &[String],
        max_tokens: usize,
        limits: RouteLimits,
    ) -> Result<RouteTaskResponse> {
        let started = Instant::now();
        let ws = probe(cwd);
        let phase = infer_phase(user_message);
        let scope_key = ws.repo_root.clone().unwrap_or_default();

        let cache_key = CacheKey {
            scope_key: scope_key.clone(),
            phase: phase.clone(),
            open_files_fp: fingerprint_open_files(open_files),
            query_fp: fingerprint_query(user_message),
            index_version: self.store.get_index_version(),
        };

        if let Some(mut cached) = self.cache.get(&cache_key) {
            cached.latency_ms = started.elapsed().as_millis() as u64;
            return Ok(cached);
        }

        let query = format!("{} {}", user_message, ws.tags.join(" "));
        let query_emb = self.embedder.embed_one(&query)?;
        let scored = self.store.score_items(
            &query,
            &query_emb,
            ws.repo_root.as_deref(),
            &ws.tags,
            agent_boost_keywords(user_message),
        )?;

        let mut resp = build_route_response(scored, &limits, &phase, max_tokens);
        resp.cache_hit = false;
        resp.latency_ms = started.elapsed().as_millis() as u64;
        resp.log_id = Uuid::new_v4().to_string();

        self.cache.put(cache_key, resp.clone());
        Ok(resp)
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
        let query_emb = self.embedder.embed_one(&query)?;
        let scored = self.store.score_items(
            &query,
            &query_emb,
            ws.repo_root.as_deref(),
            &ws.tags,
            false,
        )?;

        let mut items = Vec::new();
        let mut tokens_used = 0;
        let mut truncated = false;

        for item in scored {
            if !include_types.contains(&item.item_type) {
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
    scored: Vec<ScoredItem>,
    limits: &RouteLimits,
    phase: &str,
    max_tokens: usize,
) -> RouteTaskResponse {
    let mut agents = Vec::new();
    let mut skills = Vec::new();
    let mut rules = Vec::new();
    let mut memory = Vec::new();
    let mut must_apply = Vec::new();
    let mut tokens_used = 0;

    for item in scored {
        let (bucket, rec_tokens) = match item.item_type {
            ItemType::Agent if agents.len() < limits.agents => {
                let rec = AgentRec {
                    name: item.topic.clone(),
                    path: item.source_path.clone().unwrap_or_default(),
                    rationale: rationale_for(&item, phase),
                    score: item.score,
                };
                let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
                agents.push(rec);
                ("agent", t)
            }
            ItemType::Skill if skills.len() < limits.skills => {
                let rec = SkillRec {
                    name: item.topic.clone(),
                    path: item.source_path.clone().unwrap_or_default(),
                    rationale: rationale_for(&item, phase),
                    score: item.score,
                };
                let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
                skills.push(rec);
                ("skill", t)
            }
            ItemType::Rule if rules.len() < limits.rules => {
                let rec = RuleRec {
                    topic: item.topic.clone(),
                    text: item.text.chars().take(300).collect(),
                    source_path: item.source_path.clone().unwrap_or_default(),
                    score: item.score,
                };
                let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
                rules.push(rec);
                ("rule", t)
            }
            ItemType::Memory if memory.len() < limits.memory => {
                if item.text.to_lowercase().contains("do not") {
                    must_apply.push(MustApply {
                        topic: item.topic.clone(),
                        text: item.text.chars().take(200).collect(),
                    });
                }
                let rec = MemoryRec {
                    topic: item.topic.clone(),
                    text: item.text.chars().take(300).collect(),
                    score: item.score,
                };
                let t = estimate_json_tokens(&serde_json::to_value(&rec).unwrap_or_default());
                memory.push(rec);
                ("memory", t)
            }
            _ => continue,
        };

        if tokens_used + rec_tokens > max_tokens {
            break;
        }
        tokens_used += rec_tokens;
        let _ = bucket;
    }

    RouteTaskResponse {
        recommended_agents: agents,
        recommended_skills: skills,
        applicable_rules: rules,
        relevant_memory: memory,
        must_apply,
        recommended_phase: phase.to_string(),
        tokens_used,
        tokens_budget: max_tokens,
        cache_hit: false,
        latency_ms: 0,
        log_id: String::new(),
    }
}

fn rationale_for(item: &ScoredItem, phase: &str) -> String {
    format!(
        "Matched {} for {} phase (score {:.2}).",
        item.topic, phase, item.score
    )
}
