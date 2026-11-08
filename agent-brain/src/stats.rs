//! Operator-facing product metrics from retrieval_log + index composition.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::adoption::{self, AdoptionMilestones};
use crate::config::Config;
use crate::db::store::{
    BrainStore, CountBucket, DailyUsage, RetrievalStats, ToolUsage, VerifyCommand,
};

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
    pub daily: Vec<DailyUsage>,
    pub latency: Vec<CountBucket>,
    pub tools: Vec<ToolUsage>,
    pub verify_commands: Vec<VerifyCommand>,
    pub agstack: AgstackUsage,
}

pub const AGSTACK_SKILLS: [&str; 2] = ["agstack-mode", "verification"];

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgstackUsage {
    pub routes: u64,
    pub routes_with_agstack_skill: u64,
    pub by_skill: BTreeMap<String, u64>,
}

#[derive(Deserialize)]
struct LoggedItem {
    #[serde(rename = "type")]
    item_type: String,
    topic: String,
}

pub fn agstack_usage(items_json: &[String]) -> AgstackUsage {
    let mut usage = AgstackUsage {
        routes: items_json.len() as u64,
        ..AgstackUsage::default()
    };
    for json in items_json {
        let items: Vec<LoggedItem> = serde_json::from_str(json).unwrap_or_default();
        let mut surfaced = false;
        for skill in AGSTACK_SKILLS {
            if items
                .iter()
                .any(|item| item.item_type == "skill" && item.topic == skill)
            {
                *usage.by_skill.entry(skill.to_string()).or_insert(0) += 1;
                surfaced = true;
            }
        }
        if surfaced {
            usage.routes_with_agstack_skill += 1;
        }
    }
    usage
}

pub fn skill_mix(usage: &AgstackUsage) -> String {
    if usage.by_skill.is_empty() {
        return "none".to_string();
    }
    usage
        .by_skill
        .iter()
        .map(|(skill, count)| format!("{skill} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn retrieval_items_since(store: &BrainStore, since_ms: i64) -> Result<Vec<String>> {
    store.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT items_returned FROM retrieval_log WHERE timestamp >= ?1 AND phase <> 'upstream_call'",
        )?;
        let rows = stmt
            .query_map([since_ms], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn collect(store: &BrainStore, config: &Config, period_days: u32) -> Result<OperatorStats> {
    adoption::ensure_installed_at(store)?;
    let now = chrono::Utc::now().timestamp_millis();
    let since = now - i64::from(period_days) * 24 * 3600 * 1000;
    let _ = crate::tool_events::ingest_hook_events_since(store, &config.home, since);
    let period = store.retrieval_stats_since(since)?;
    let daily = store.daily_usage_since(since)?;
    let latency = store.latency_buckets_since(since)?;
    let tools = store.tool_usage_since(since)?;
    let verify_commands = store.verify_commands_since(since)?;
    let agstack = agstack_usage(&retrieval_items_since(store, since)?);
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
        daily,
        latency,
        tools,
        verify_commands,
        agstack,
    })
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
        out.push('\n');
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
        out.push_str("- No routes logged yet (send an agent turn with route_task)\n");
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
    if stats.agstack.routes > 0 {
        out.push_str(&format!(
            "- agstack skills surfaced: {} of {} routes ({})\n",
            stats.agstack.routes_with_agstack_skill,
            stats.agstack.routes,
            skill_mix(&stats.agstack)
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
        stats.value.memories_committed, stats.value.blocked_full_reads,
    ));
    if stats.period.route_calls == 0 {
        out.push_str(
            "- Tip: run Agent mode once so route_task logs activity, then: `agent-brain stats`\n",
        );
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

pub fn format_markdown(stats: &OperatorStats) -> String {
    let p = &stats.period;
    let routed_tokens: u64 = stats.daily.iter().map(|d| d.routed_tokens).sum();
    let verify_passed: u64 = stats.daily.iter().map(|d| d.verify_passed).sum();
    let verify_failed: u64 = stats.daily.iter().map(|d| d.verify_failed).sum();
    let verify_runs = verify_passed + verify_failed;
    let pass_rate = if verify_runs == 0 {
        "no runs".to_string()
    } else {
        format!("{:.1}%", verify_passed as f64 / verify_runs as f64 * 100.0)
    };
    let mut out = format!(
        "**agent-brain summary, last {} days (measured)**\n\n| Measure | Value |\n|---|---|\n",
        stats.period_days
    );
    out.push_str(&format!("| Routed turns | {} |\n", p.route_calls));
    out.push_str(&format!("| Context tokens returned | {routed_tokens} |\n"));
    out.push_str(&format!(
        "| Average tokens per turn | {:.0} |\n",
        p.avg_routed_tokens
    ));
    out.push_str(&format!(
        "| Cache hit rate | {:.1}% |\n",
        p.cache_hit_rate * 100.0
    ));
    out.push_str(&format!(
        "| Latency avg / p95 | {:.0} ms / {} ms |\n",
        p.avg_latency_ms, p.p95_latency_ms
    ));
    out.push_str(&format!(
        "| Turns with must_apply constraints | {} |\n",
        p.routes_with_constraints
    ));
    out.push_str(&format!("| Bounded tool calls | {} |\n", p.tool_calls));
    out.push_str(&format!(
        "| agstack skills surfaced in routes | {} of {} ({}) |\n",
        stats.agstack.routes_with_agstack_skill,
        stats.agstack.routes,
        skill_mix(&stats.agstack)
    ));
    out.push_str(&format!(
        "| Full-read steers | {} |\n",
        stats.value.blocked_full_reads
    ));
    out.push_str(&format!(
        "| Verify runs passed / failed | {verify_passed} / {verify_failed} |\n"
    ));
    out.push_str(&format!("| Verify pass rate | {pass_rate} |\n"));
    out.push_str("\nFrom the local database only. Estimated savings are left out.\n");
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
