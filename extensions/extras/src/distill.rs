use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct DistilledArch {
    pub title: String,
    pub generated_at: String,
    pub total_facts: usize,
    pub system_overview: Vec<String>,
    pub key_modules: Vec<ModuleSummary>,
    pub decisions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ModuleSummary {
    pub name: String,
    pub description: String,
    pub score: f64,
    pub fact_count: usize,
}

pub fn distill(store: &agent_brain::db::store::BrainStore) -> Result<DistilledArch> {
    let rows = store.list_export_facts()?;
    let total_facts = rows.len();

    let mut by_topic: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for row in &rows {
        let topic = row["topic"].as_str().unwrap_or("unknown").to_string();
        by_topic.entry(topic).or_default().push(row.clone());
    }

    let mut topic_scores: Vec<(String, f64, usize)> = by_topic
        .into_iter()
        .map(|(topic, facts)| {
            let n = facts.len();
            let sum: f64 = facts.iter().filter_map(|f| f["confidence"].as_f64()).sum();
            let avg_conf = if n > 0 { sum / n as f64 } else { 0.0 };
            (topic, avg_conf, n)
        })
        .collect();
    topic_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let system_overview = topic_scores
        .iter()
        .take(5)
        .map(|(topic, _, _)| format!("- **{}**: key subsystem", topic))
        .collect();

    let key_modules: Vec<ModuleSummary> = topic_scores
        .iter()
        .take(10)
        .map(|(name, score, count)| ModuleSummary {
            name: name.clone(),
            description: format!("Active topic with {} facts", count),
            score: *score,
            fact_count: *count,
        })
        .collect();

    let decisions = vec![
        "ADD-only memory model (no UPDATE, only invalidation)".to_string(),
        "Bundled SQLite over external Postgres — zero-config, local-first".to_string(),
    ];

    Ok(DistilledArch {
        title: "Architecture — agent-brain".to_string(),
        generated_at: Utc::now().to_rfc3339(),
        total_facts,
        system_overview,
        key_modules,
        decisions,
    })
}

pub fn write_architecture_md(distilled: &DistilledArch, path: &Path) -> Result<()> {
    let mut md = String::new();
    md.push_str(&format!("# {}\n\n", distilled.title));
    md.push_str(&format!(
        "*Auto-generated from {} active facts. Last updated: {}*\n\n",
        distilled.total_facts, distilled.generated_at
    ));
    md.push_str("## System Overview\n\n");
    for line in &distilled.system_overview {
        md.push_str(line);
        md.push('\n');
    }
    md.push_str("\n## Key Modules\n\n");
    for m in &distilled.key_modules {
        md.push_str(&format!(
            "- **{}** — {} (confidence: {:.2}, {} facts)\n",
            m.name, m.description, m.score, m.fact_count
        ));
    }
    md.push_str("\n## Decisions\n\n");
    for d in &distilled.decisions {
        md.push_str(&format!("- {}\n", d));
    }
    std::fs::write(path, md)?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClusterStats {
    pub clusters_found: usize,
    pub items_clustered: usize,
    pub facts_created: usize,
    pub items_superseded: usize,
    pub dry_run: bool,
    pub threshold: f64,
}

pub fn cluster_and_summarize(
    store: &agent_brain::db::store::BrainStore,
    threshold: f64,
    dry_run: bool,
) -> Result<ClusterStats> {
    let items = store.load_searchable_items()?;
    let mut by_topic: HashMap<String, Vec<(usize, &agent_brain::db::store::SearchRow)>> =
        HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        if item.embedding.is_none() {
            continue;
        }
        by_topic
            .entry(item.topic.clone())
            .or_default()
            .push((idx, item));
    }

    let mut clusters_found = 0usize;
    let mut items_clustered = 0usize;
    let mut facts_created = 0usize;
    let mut items_superseded = 0usize;

    for (_topic, group) in &by_topic {
        if group.len() < 2 {
            continue;
        }

        let embeds: Vec<Vec<f32>> = group
            .iter()
            .map(|(_, item)| agent_brain::db::store::bytes_to_f32(item.embedding.as_ref().unwrap()))
            .collect();

        let n = group.len();
        let mut parent: Vec<usize> = (0..n).collect();

        fn find(parent: &mut [usize], x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find(parent, parent[x]);
            }
            parent[x]
        }
        fn union(parent: &mut [usize], a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra != rb {
                parent[ra] = rb;
            }
        }

        for i in 0..n {
            for j in (i + 1)..n {
                let sim = agent_brain::embed::cosine(&embeds[i], &embeds[j]);
                if sim >= threshold {
                    union(&mut parent, i, j);
                }
            }
        }

        let mut cluster_map: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            let root = find(&mut parent, i);
            cluster_map.entry(root).or_default().push(i);
        }

        for (_root, members) in &cluster_map {
            if members.len() < 2 {
                continue;
            }
            clusters_found += 1;
            items_clustered += members.len();

            if dry_run {
                continue;
            }

            let member_rows: Vec<&agent_brain::db::store::SearchRow> =
                members.iter().map(|&i| group[i].1).collect();

            let combined: Vec<&str> = member_rows
                .iter()
                .map(|r| r.text.as_str())
                .filter(|t| !t.is_empty())
                .collect();
            let combined_text = if combined.is_empty() {
                "auto-merged cluster".to_string()
            } else if combined.len() == 1 {
                combined[0].to_string()
            } else {
                combined.join("; ")
            };

            let rows = store.list_facts(10_000)?;
            let max_conf = member_rows
                .iter()
                .filter_map(|r| {
                    rows.iter()
                        .find(|f| f.get("topic").and_then(|v| v.as_str()) == Some(r.topic.as_str()))
                })
                .filter_map(|f| f.get("confidence").and_then(|v| v.as_f64()))
                .fold(0.95f64, |acc, c| acc.max(c));

            let result = store.store_fact(
                &member_rows[0].topic,
                &combined_text,
                &member_rows[0].scope,
                member_rows[0].scope_key.as_deref(),
                max_conf.max(0.95),
                "distill",
                "",
                &[],
                "positive",
            );

            match result {
                Ok(_meta) => {
                    facts_created += 1;
                    for row in &member_rows {
                        if store.invalidate_fact(&row.id).is_ok() {
                            items_superseded += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to store cluster fact: {}", e);
                }
            }
        }
    }

    Ok(ClusterStats {
        clusters_found,
        items_clustered,
        facts_created,
        items_superseded,
        dry_run,
        threshold,
    })
}
