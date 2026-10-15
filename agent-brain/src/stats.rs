//! Operator-facing product metrics from retrieval_log + index composition.

use std::collections::BTreeMap;
#[cfg(feature = "extras")]
use std::fs;
#[cfg(feature = "extras")]
use std::path::Path;

use anyhow::Result;
#[cfg(feature = "extras")]
use anyhow::Context;
use serde::Serialize;

use crate::adoption::{self, AdoptionMilestones};
use crate::config::Config;
use crate::db::store::{BrainStore, RetrievalStats};

#[derive(Debug, Clone, Serialize)]
pub struct IndexBreakdown {
    pub total_indexed: usize,
    pub active_memories: usize,
    pub by_type: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValueSummary {
    pub memories_committed: usize,
    pub blocked_full_reads: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperatorStats {
    pub period_days: u32,
    pub index: IndexBreakdown,
    pub period: RetrievalStats,
    pub value: ValueSummary,
    pub adoption: AdoptionMilestones,
}

pub fn collect(store: &BrainStore, config: &Config, period_days: u32) -> Result<OperatorStats> {
    adoption::ensure_installed_at(store)?;
    let now = chrono::Utc::now().timestamp_millis();
    let since = now - i64::from(period_days) * 24 * 3600 * 1000;
    let _ = crate::tool_events::ingest_hook_events_since(store, &config.home, since);
    let period = store.retrieval_stats_since(since)?;
    let mut by_type = BTreeMap::new();
    for (item_type, count) in store.count_indexed_by_type()? {
        by_type.insert(item_type, count);
    }
    let index = IndexBreakdown {
        total_indexed: store.count_indexed_items()?,
        active_memories: store.count_active_facts()?,
        by_type,
    };
    let adoption = adoption::load_milestones(store)?;
    let value = ValueSummary {
        memories_committed: store.count_facts_created_since(since)?,
        blocked_full_reads: period.inefficient_read_events,
    };
    Ok(OperatorStats {
        period_days,
        index,
        period,
        value,
        adoption,
    })
}

#[cfg(feature = "extras")]
pub fn persist_proof_snapshot(home: &Path, report: &crate::proofs::ProofReport) -> Result<()> {
    let dir = home.join("metrics");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join("proof-latest.json");
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&path, format!("{json}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn format_text(stats: &OperatorStats) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# agent-brain stats ({} days)\n\n",
        stats.period_days
    ));

    out.push_str("## Index\n\n");
    out.push_str(&format!(
        "- Indexed items: {}\n- Active memories: {}\n",
        stats.index.total_indexed, stats.index.active_memories
    ));
    if !stats.index.by_type.is_empty() {
        out.push_str("\n");
        for (item_type, count) in &stats.index.by_type {
            out.push_str(&format!("- {item_type}: {count}\n"));
        }
    }

    out.push_str("\n## Routing (period)\n\n");
    out.push_str(&format!(
        "- Route calls: {}\n- Cache hit rate: {:.1}%\n- Avg latency: {:.0}ms · P95: {}ms\n",
        stats.period.route_calls,
        stats.period.cache_hit_rate * 100.0,
        stats.period.avg_latency_ms,
        stats.period.p95_latency_ms
    ));
    if stats.period.route_calls > 0 {
        out.push_str(&format!(
            "- Avg routed tokens: {:.0} (budgeted context returned to the agent)\n",
            stats.period.avg_routed_tokens
        ));
    } else {
        out.push_str(
            "- No routes logged yet (send an agent turn with route_task)\n",
        );
    }
    if stats.period.routes_with_constraints > 0 {
        out.push_str(&format!(
            "- Supervisor: {} routes enforced must_apply ({} constraints total)\n",
            stats.period.routes_with_constraints, stats.period.total_must_apply
        ));
    }
    if stats.period.tool_calls > 0 {
        out.push_str(&format!(
            "- Token tools: {} bounded-read calls\n",
            stats.period.tool_calls
        ));
    }
    if stats.period.inefficient_read_events > 0 {
        out.push_str(&format!(
            "- Inefficient Read steers: {} (use grep_search / read_file_head)\n",
            stats.period.inefficient_read_events
        ));
    }

    out.push_str("\n## Activity (period)\n\n");
    out.push_str(&format!(
        "- Memories committed: {}\n- Full-read steers: {}\n",
        stats.value.memories_committed,
        stats.value.blocked_full_reads,
    ));
    if stats.period.route_calls == 0 {
        out.push_str("- Tip: run Agent mode once so route_task logs activity, then: `agent-brain stats`\n");
    }

    out.push_str("\n## Install timeline (local)\n\n");
    out.push_str(&format!(
        "- Installed: {}\n- First route: {}\n- Starter pack: {}\n- Supervisor pack: {}\n",
        stats.adoption.installed_at.as_deref().unwrap_or("—"),
        stats.adoption.first_route_at.as_deref().unwrap_or("—"),
        stats.adoption.starter_pack_at.as_deref().unwrap_or("—"),
        stats.adoption.supervisor_pack_at.as_deref().unwrap_or("—"),
    ));


    out
}

/// One-line summary for doctor / onboarding.
pub fn format_summary_line(stats: &OperatorStats) -> String {
    if stats.period.route_calls == 0 {
        return format!(
            "Index: {} items · no routes in last {}d — run Agent mode once, then: agent-brain stats",
            stats.index.total_indexed,
            stats.period_days
        );
    }
    let routed = if stats.period.route_calls > 0 {
        format!("~{:.0} tok avg routed", stats.period.avg_routed_tokens)
    } else {
        "no routes yet".into()
    };
    let supervisor = if stats.period.routes_with_constraints > 0 {
        format!(
            " · must_apply on {} routes",
            stats.period.routes_with_constraints
        )
    } else {
        String::new()
    };
    format!(
        "{} routes ({}d) · {}{} · p95 {}ms · index {}",
        stats.period.route_calls,
        stats.period_days,
        routed,
        supervisor,
        stats.period.p95_latency_ms,
        stats.index.total_indexed
    )
}
