use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
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

pub fn distill(store: &crate::db::store::BrainStore) -> Result<DistilledArch> {
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
            let sum: f64 = facts
                .iter()
                .filter_map(|f| f["confidence"].as_f64())
                .sum();
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
