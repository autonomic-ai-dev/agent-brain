//! Operator-facing digests from retrieval_log and context_weights.

use anyhow::Result;
use serde::Serialize;

use crate::db::store::BrainStore;

#[derive(Debug, Serialize)]
pub struct WeeklyDigest {
    pub period_days: u32,
    pub route_calls: usize,
    pub upstream_calls: usize,
    pub cache_hit_rate: f64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: u64,
    pub phases: Vec<PhaseCount>,
    pub routes_with_savings: usize,
    pub avg_saved_pct: f64,
    pub total_saved_tokens: u64,
    pub avg_routed_tokens: f64,
    pub routes_with_constraints: usize,
    pub total_must_apply: usize,
    pub feedback: FeedbackSummary,
}

#[derive(Debug, Serialize)]
pub struct PhaseCount {
    pub phase: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct FeedbackSummary {
    pub items_tracked: usize,
    pub total_useful: i64,
    pub total_useless: i64,
    pub lowest_weight: Vec<WeightRow>,
}

#[derive(Debug, Serialize)]
pub struct WeightRow {
    pub item_id: String,
    pub weight: f64,
    pub useful_count: i64,
    pub useless_count: i64,
}

pub fn weekly_digest(store: &BrainStore, days: u32) -> Result<WeeklyDigest> {
    let now = chrono::Utc::now().timestamp_millis();
    let since = now - i64::from(days) * 24 * 3600 * 1000;
    let stats = store.retrieval_stats_since(since)?;
    let feedback = store.context_feedback_summary(5)?;

    Ok(WeeklyDigest {
        period_days: days,
        route_calls: stats.route_calls,
        upstream_calls: stats.upstream_calls,
        cache_hit_rate: stats.cache_hit_rate,
        avg_latency_ms: stats.avg_latency_ms,
        p95_latency_ms: stats.p95_latency_ms,
        phases: stats
            .phases
            .into_iter()
            .map(|(phase, count)| PhaseCount { phase, count })
            .collect(),
        routes_with_savings: stats.routes_with_savings,
        avg_saved_pct: stats.avg_saved_pct,
        total_saved_tokens: stats.total_saved_tokens,
        avg_routed_tokens: stats.avg_routed_tokens,
        routes_with_constraints: stats.routes_with_constraints,
        total_must_apply: stats.total_must_apply,
        feedback: FeedbackSummary {
            items_tracked: feedback.items_tracked,
            total_useful: feedback.total_useful,
            total_useless: feedback.total_useless,
            lowest_weight: feedback
                .lowest_weight
                .into_iter()
                .map(|r| WeightRow {
                    item_id: r.item_id,
                    weight: r.weight,
                    useful_count: r.useful_count,
                    useless_count: r.useless_count,
                })
                .collect(),
        },
    })
}

pub fn format_weekly_digest(digest: &WeeklyDigest) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# agent-brain weekly digest ({} days)\n\n",
        digest.period_days
    ));
    out.push_str(&format!(
        "- Route calls: {}\n- Upstream calls: {}\n- Cache hit rate: {:.1}%\n- Avg latency: {:.0}ms\n- P95 latency: {}ms\n",
        digest.route_calls,
        digest.upstream_calls,
        digest.cache_hit_rate * 100.0,
        digest.avg_latency_ms,
        digest.p95_latency_ms
    ));
    if digest.routes_with_savings > 0 {
        out.push_str(&format!(
            "- Token savings: ~{:.0}% avg ({} routes) · ~{} tok saved (est.)\n- Avg routed tokens: {:.0}\n",
            digest.avg_saved_pct,
            digest.routes_with_savings,
            digest.total_saved_tokens,
            digest.avg_routed_tokens
        ));
    }
    if digest.routes_with_constraints > 0 {
        out.push_str(&format!(
            "- Supervisor constraints: {} routes with must_apply · {} total constraints\n",
            digest.routes_with_constraints, digest.total_must_apply
        ));
    }
    out.push('\n');
    if !digest.phases.is_empty() {
        out.push_str("## Phases\n\n");
        for p in &digest.phases {
            out.push_str(&format!("- {}: {}\n", p.phase, p.count));
        }
        out.push('\n');
    }
    out.push_str("## Context feedback\n\n");
    out.push_str(&format!(
        "- Items tracked: {}\n- Useful signals: {}\n- Useless signals: {}\n",
        digest.feedback.items_tracked, digest.feedback.total_useful, digest.feedback.total_useless
    ));
    if !digest.feedback.lowest_weight.is_empty() {
        out.push_str("\n### Lowest weight items\n\n");
        for row in &digest.feedback.lowest_weight {
            out.push_str(&format!(
                "- {} weight={:.2} (+{} / -{})\n",
                row.item_id, row.weight, row.useful_count, row.useless_count
            ));
        }
    }
    out
}
