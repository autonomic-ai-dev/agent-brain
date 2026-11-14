//! Observation engine (Zep-inspired): synthesize higher-level must_apply candidates from recurring facts.

use anyhow::Result;
use serde::Serialize;

use crate::db::store::{content_hash, word_count, BrainStore, RecurringMemoryTopic};
use crate::embed::Embedder;

#[derive(Debug, Clone, Serialize)]
pub struct ObservationReport {
    pub dry_run: bool,
    pub candidates: usize,
    pub synthesized: usize,
    pub skipped_existing: usize,
    pub topics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ObservationConfig {
    pub min_facts_per_topic: usize,
    pub window_days: u32,
}

impl Default for ObservationConfig {
    fn default() -> Self {
        Self {
            min_facts_per_topic: 3,
            window_days: 90,
        }
    }
}

pub fn observation_topic(base: &str) -> String {
    format!("obs/{base}")
}

pub fn run_observations(
    store: &BrainStore,
    embedder: &Embedder,
    cfg: &ObservationConfig,
    dry_run: bool,
) -> Result<ObservationReport> {
    let now = chrono::Utc::now().timestamp_millis();
    let _ = store.prune_expired_facts()?;

    let since_ms = now - i64::from(cfg.window_days) * 24 * 3600 * 1000;
    let groups = store.list_recurring_memory_topics(cfg.min_facts_per_topic, since_ms)?;

    let mut synthesized = 0usize;
    let mut skipped_existing = 0usize;
    let mut topics = Vec::new();

    for group in groups {
        let obs_topic = observation_topic(&group.topic);
        let fact = synthesize_fact(&group);
        if word_count(&fact) > 50 {
            continue;
        }
        let hash = content_hash(&fact);
        if store.fact_exists_by_hash(&hash, &group.scope, group.scope_key.as_deref())? {
            skipped_existing += 1;
            continue;
        }
        topics.push(obs_topic.clone());
        if dry_run {
            synthesized += 1;
            continue;
        }

        let apply_when = if group.apply_when_tags.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&group.apply_when_tags)?)
        };
        let polarity = if group.negative_count * 2 >= group.fact_count {
            "negative"
        } else {
            "positive"
        };
        let embedding = embedder.embed_one(&format!("{obs_topic} {fact}"))?;
        let res = store.store_fact_full(
            &obs_topic,
            &fact,
            &group.scope,
            group.scope_key.as_deref(),
            0.92,
            "observation",
            &hash,
            &embedding,
            polarity,
            apply_when.as_deref(),
            None,
        )?;
        link_observation_sources(store, &res.id, &group)?;
        synthesized += 1;
    }

    Ok(ObservationReport {
        dry_run,
        candidates: topics.len() + skipped_existing,
        synthesized,
        skipped_existing,
        topics,
    })
}

fn synthesize_fact(group: &RecurringMemoryTopic) -> String {
    let snippet: String = group.latest_fact.chars().take(120).collect();
    format!(
        "Recurring pattern ({} facts on '{}'): {}",
        group.fact_count, group.topic, snippet
    )
}

fn link_observation_sources(
    store: &BrainStore,
    obs_fact_id: &str,
    group: &RecurringMemoryTopic,
) -> Result<()> {
    let source_ids = store.list_active_fact_ids_for_topic(
        &group.topic,
        &group.scope,
        group.scope_key.as_deref(),
    )?;
    for source_id in source_ids {
        if source_id == obs_fact_id {
            continue;
        }
        store.insert_fact_lineage(obs_fact_id, "fact", &source_id, "synthesized_from")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::BrainStore;
    use crate::embed::Embedder;
    use tempfile::TempDir;

    fn test_store() -> (TempDir, BrainStore, Embedder) {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("brain.db");
        let store = BrainStore::open(&db).unwrap();
        let embedder = Embedder::deterministic();
        (dir, store, embedder)
    }

    #[test]
    fn synthesizes_observation_after_repeated_topic() {
        let (_dir, store, embedder) = test_store();
        let emb = vec![0.02; 384];
        for i in 0..3 {
            let fact = format!("Use Vitest for unit tests variant {i}");
            store
                .store_fact_full(
                    "testing",
                    &fact,
                    "project",
                    None,
                    0.9,
                    "agent",
                    &content_hash(&fact),
                    &emb,
                    "positive",
                    Some(r#"["phase:implementing"]"#),
                    None,
                )
                .unwrap();
        }

        let report =
            run_observations(&store, &embedder, &ObservationConfig::default(), false).unwrap();
        assert_eq!(report.synthesized, 1);
        assert!(report.topics.iter().any(|t| t == "obs/testing"));

        let facts = store.list_facts(10).unwrap();
        assert!(facts.iter().any(|f| f["topic"] == "obs/testing"));
    }
}
