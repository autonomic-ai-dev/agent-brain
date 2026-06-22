use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;

use crate::types::RouteTaskResponse;

/// Rough tokens if every indexed item were loaded (~120 tok/item). Informational only.
pub const NAIVE_TOKENS_PER_INDEXED_ITEM: usize = 120;

#[derive(Debug, Clone, Copy)]
pub struct TokenSavings {
    pub index_total: usize,
    pub naive_tokens: usize,
    pub routed_tokens: usize,
    pub tokens_budget: usize,
    pub saved_tokens: usize,
    pub saved_pct: usize,
}

pub fn token_savings(resp: &RouteTaskResponse) -> Option<TokenSavings> {
    if resp.index_total == 0 {
        return None;
    }
    let naive_tokens = resp
        .index_total
        .saturating_mul(NAIVE_TOKENS_PER_INDEXED_ITEM);
    let saved_tokens = naive_tokens.saturating_sub(resp.tokens_used);
    let saved_pct = if naive_tokens > 0 {
        saved_tokens.saturating_mul(100) / naive_tokens
    } else {
        0
    };
    Some(TokenSavings {
        index_total: resp.index_total,
        naive_tokens,
        routed_tokens: resp.tokens_used,
        tokens_budget: resp.tokens_budget,
        saved_tokens,
        saved_pct,
    })
}

pub fn format_token_savings_line(resp: &RouteTaskResponse) -> String {
    let Some(s) = token_savings(resp) else {
        return String::new();
    };
    format!(
        "Index: {} items · est. naive load ~{} tok · routed {}/{} tok · saved ~{} tok (~{}%)",
        s.index_total,
        s.naive_tokens,
        s.routed_tokens,
        s.tokens_budget,
        s.saved_tokens,
        s.saved_pct
    )
}

pub fn format_briefing(resp: &RouteTaskResponse) -> String {
    let mut out = String::new();
    let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
    out.push_str(&format!("# agent-brain route ({ts})\n\n"));
    out.push_str(&format!(
        "Phase: **{}** · {}ms · cache: {} · tokens: {}/{}\n",
        resp.recommended_phase,
        resp.latency_ms,
        if resp.cache_hit { "hit" } else { "miss" },
        resp.tokens_used,
        resp.tokens_budget
    ));
    let savings = format_token_savings_line(resp);
    if !savings.is_empty() {
        out.push_str(&format!("{savings}\n"));
    }
    if !resp.must_apply.is_empty() {
        out.push_str(&format!(
            "**Supervisor:** {} constraint(s) in must_apply — follow before using tools\n",
            resp.must_apply.len()
        ));
    }
    if !resp.suggested_native_tools.is_empty() {
        let tools: Vec<_> = resp
            .suggested_native_tools
            .iter()
            .map(|t| t.tool.as_str())
            .collect();
        out.push_str(&format!(
            "**Token tools:** prefer {} before full Read/cat\n",
            tools.join(", ")
        ));
    }
    if !resp.log_id.is_empty() {
        out.push_str(&format!("Log: `{}`\n", resp.log_id));
    }
    if let Some(repo) = &resp.repo_snapshot {
        out.push_str(&format!("**Repo:** {repo}\n"));
    }
    out.push('\n');

    section_list(
        &mut out,
        "Agents",
        resp.recommended_agents
            .iter()
            .map(|a| format!("**{}** — {} _(score {:.2})_", a.name, a.rationale, a.score)),
    );
    section_list(
        &mut out,
        "Skills",
        resp.recommended_skills
            .iter()
            .map(|s| format!("**{}** — {} _(score {:.2})_", s.name, s.rationale, s.score)),
    );
    section_list(
        &mut out,
        "Rules",
        resp.applicable_rules.iter().map(|r| {
            let preview: String = r.text.chars().take(80).collect();
            let ellipsis = if r.text.chars().count() > 80 {
                "…"
            } else {
                ""
            };
            format!(
                "**{}** — {}{} _(score {:.2})_",
                r.topic, preview, ellipsis, r.score
            )
        }),
    );
    section_list(
        &mut out,
        "Memory",
        resp.relevant_memory
            .iter()
            .map(|m| format!("**{}** — {} _(score {:.2})_", m.topic, m.text, m.score)),
    );
    section_list(
        &mut out,
        "Must apply",
        resp.must_apply
            .iter()
            .map(|m| format!("**{}** — {}", m.topic, m.text)),
    );

    out
}

fn section_list(out: &mut String, title: &str, items: impl Iterator<Item = String>) {
    let rows: Vec<String> = items.collect();
    out.push_str(&format!("## {title} ({})\n", rows.len()));
    if rows.is_empty() {
        out.push_str("_none_\n\n");
        return;
    }
    for row in rows {
        out.push_str(&format!("- {row}\n"));
    }
    out.push('\n');
}

pub fn format_summary_line(resp: &RouteTaskResponse) -> String {
    let skill_names = join_top_names(resp.recommended_skills.iter().map(|s| s.name.as_str()), 3);
    let agent_names = join_top_names(resp.recommended_agents.iter().map(|a| a.name.as_str()), 2);
    let savings = token_savings(resp).map(|s| format!(" · saved ~{}%", s.saved_pct));
    let constraints = if resp.must_apply.is_empty() {
        String::new()
    } else {
        let topics = join_top_names(resp.must_apply.iter().map(|m| m.topic.as_str()), 2);
        format!(" · must_apply: {} [{topics}]", resp.must_apply.len())
    };
    let native_tools = if resp.suggested_native_tools.is_empty() {
        String::new()
    } else {
        let names = join_top_names(
            resp.suggested_native_tools.iter().map(|t| t.tool.as_str()),
            2,
        );
        format!(" · tools: [{names}]")
    };
    let read_gate = if read_gate_mode() == "off" {
        String::new()
    } else {
        format!(" · read_gate={}", read_gate_mode())
    };
    let repo = resp
        .repo_snapshot
        .as_deref()
        .map(|s| format!(" · repo: {s}"))
        .unwrap_or_default();
    format!(
        "phase={} · skills: {} [{skill_names}] · agents: {} [{agent_names}] · {} rules · {} memory · {}/{} tok{}{}{}{}{} · {}ms · log={} · {}",
        resp.recommended_phase,
        resp.recommended_skills.len(),
        resp.recommended_agents.len(),
        resp.applicable_rules.len(),
        resp.relevant_memory.len(),
        resp.tokens_used,
        resp.tokens_budget,
        savings.unwrap_or_default(),
        constraints,
        native_tools,
        read_gate,
        repo,
        resp.latency_ms,
        resp.log_id,
        briefing_path_display()
    )
}

fn join_top_names<'a>(names: impl Iterator<Item = &'a str>, max: usize) -> String {
    let picked: Vec<&str> = names.take(max).collect();
    if picked.is_empty() {
        "—".into()
    } else {
        picked.join(", ")
    }
}

pub fn format_stderr_line(resp: &RouteTaskResponse) -> String {
    format!("agent-brain: {}", format_summary_line(resp))
}

fn briefing_path_display() -> String {
    if let Ok(home) = std::env::var("AGENT_BRAIN_HOME") {
        return PathBuf::from(home)
            .join("logs/last-route.md")
            .display()
            .to_string();
    }
    crate::global_workspace::memory_logs_dir()
        .join("last-route.md")
        .display()
        .to_string()
}

pub fn publish_briefing(
    home: &Path,
    logs_dir: &Path,
    resp: &RouteTaskResponse,
    stderr_line: bool,
    store: Option<&crate::db::store::BrainStore>,
) {
    let mut briefing = format_briefing(resp);
    if let Some(store) = store {
        briefing.push_str(&format_supervisor_period_section(home, store));
    }
    if fs::create_dir_all(logs_dir).is_ok() {
        let path = logs_dir.join("last-route.md");
        let _ = fs::write(&path, &briefing);
    }
    publish_route_state(home, resp);
    if stderr_line {
        eprintln!("{}", format_stderr_line(resp));
    }
}

pub fn publish_route_state(home: &Path, resp: &RouteTaskResponse) {
    let hooks = home.join("hooks");
    if fs::create_dir_all(&hooks).is_err() {
        return;
    }
    let path = hooks.join("route_state.json");
    let mut state: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = state.as_object_mut() {
        obj.insert(
            "route_log_id".into(),
            serde_json::Value::String(resp.log_id.clone()),
        );
        obj.insert(
            "route_phase".into(),
            serde_json::Value::String(resp.recommended_phase.clone()),
        );
        obj.insert(
            "must_apply".into(),
            serde_json::to_value(&resp.must_apply).unwrap_or_default(),
        );
        obj.insert(
            "suggested_native_tools".into(),
            serde_json::to_value(&resp.suggested_native_tools).unwrap_or_default(),
        );
        obj.insert(
            "route_context_at".into(),
            serde_json::json!(chrono::Utc::now().timestamp()),
        );
    }
    let _ = fs::write(&path, serde_json::to_string(&state).unwrap_or_default());
}

pub fn clear_anti_pattern_suggestion(home: &Path) -> Result<()> {
    let path = home.join("hooks/route_state.json");
    let mut state: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = state.as_object_mut() {
        obj.remove("anti_pattern_suggestion");
    }
    fs::write(&path, serde_json::to_string(&state)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn read_gate_mode() -> &'static str {
    let raw = std::env::var("AGENT_BRAIN_READ_GATE").unwrap_or_else(|_| "steer".into());
    let lower = raw.trim().to_ascii_lowercase();
    if lower == "off" {
        "off"
    } else if lower == "hard" {
        "hard"
    } else {
        "steer"
    }
}

pub fn format_supervisor_period_section(
    home: &Path,
    store: &crate::db::store::BrainStore,
) -> String {
    let since = chrono::Utc::now().timestamp_millis() - 24 * 3600 * 1000;
    let _ = crate::tool_events::ingest_hook_events_since(store, home, since);
    let stats = store.retrieval_stats_since(since).ok();
    let pending = read_pending_memory_suggestion(home).is_some();
    let mut out = String::from("## Supervisor (24h)\n\n");
    out.push_str(&format!("- Read gate: **{}**\n", read_gate_mode()));
    if let Some(stats) = stats {
        if stats.tool_calls > 0 {
            out.push_str(&format!(
                "- Token tools: {} calls · ~{} tok saved · {:.0}% avg savings\n",
                stats.tool_calls, stats.tool_tokens_saved, stats.tool_avg_savings_pct
            ));
        }
        if stats.inefficient_read_events > 0 {
            out.push_str(&format!(
                "- Inefficient Read steers: {} — run `agent-brain suggest-memory approve` to persist\n",
                stats.inefficient_read_events
            ));
        }
        if stats.routes_with_constraints > 0 {
            out.push_str(&format!(
                "- must_apply routes: {} ({} constraints)\n",
                stats.routes_with_constraints, stats.total_must_apply
            ));
        }
    }
    if pending {
        out.push_str("- **Pending memory suggestion** — `agent-brain suggest-memory approve`\n");
    }
    if out.ends_with("## Supervisor (24h)\n\n") {
        out.push_str("_no supervisor telemetry in last 24h_\n");
    }
    out.push('\n');
    out
}

pub fn read_anti_pattern_suggestion(home: &Path) -> Option<serde_json::Value> {
    let path = home.join("hooks/route_state.json");
    let state: serde_json::Value = fs::read_to_string(path).ok()?.parse().ok()?;
    state.get("anti_pattern_suggestion").cloned()
}

pub fn read_edit_memory_suggestion(home: &Path) -> Option<serde_json::Value> {
    let path = home.join("hooks/route_state.json");
    let state: serde_json::Value = fs::read_to_string(path).ok()?.parse().ok()?;
    state.get("edit_memory_suggestion").cloned()
}

pub fn read_pending_memory_suggestion(home: &Path) -> Option<serde_json::Value> {
    read_anti_pattern_suggestion(home).or_else(|| read_edit_memory_suggestion(home))
}

pub fn clear_edit_memory_suggestion(home: &Path) -> Result<()> {
    let path = home.join("hooks/route_state.json");
    let mut state: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = state.as_object_mut() {
        obj.remove("edit_memory_suggestion");
    }
    fs::write(&path, serde_json::to_string(&state)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentRec, SkillRec};

    #[test]
    fn briefing_lists_routed_items() {
        let resp = RouteTaskResponse {
            recommended_agents: vec![AgentRec {
                name: "rust-reviewer".into(),
                path: "/path".into(),
                rationale: "matched".into(),
                score: 0.9,
            }],
            recommended_skills: vec![SkillRec {
                name: "rust-testing".into(),
                path: "/skill".into(),
                rationale: "matched".into(),
                score: 0.8,
                text: None,
            }],
            recommended_phase: "debugging".into(),
            tokens_used: 100,
            tokens_budget: 500,
            index_total: 2000,
            latency_ms: 12,
            log_id: "abc".into(),
            ..Default::default()
        };
        let text = format_briefing(&resp);
        assert!(text.contains("rust-reviewer"));
        assert!(text.contains("rust-testing"));
        assert!(text.contains("debugging"));
        assert!(text.contains("saved ~"));
        assert!(text.contains("2000 items"));

        let summary = format_summary_line(&resp);
        assert!(summary.contains("rust-reviewer"));
        assert!(summary.contains("rust-testing"));
        assert!(summary.contains("phase=debugging"));
        assert!(summary.contains("saved ~"));
    }

    #[test]
    fn briefing_highlights_supervisor_constraints() {
        let resp = RouteTaskResponse {
            must_apply: vec![crate::types::MustApply {
                topic: "no-read-dist".into(),
                text: "Never read dist/".into(),
            }],
            recommended_phase: "implementing".into(),
            tokens_used: 100,
            tokens_budget: 500,
            index_total: 100,
            ..Default::default()
        };
        let text = format_briefing(&resp);
        assert!(text.contains("Supervisor:"));
        let summary = format_summary_line(&resp);
        assert!(summary.contains("must_apply: 1"));
    }

    #[test]
    fn token_savings_math() {
        let resp = RouteTaskResponse {
            tokens_used: 477,
            tokens_budget: 500,
            index_total: 2000,
            ..Default::default()
        };
        let s = token_savings(&resp).unwrap();
        assert_eq!(s.naive_tokens, 240_000);
        assert_eq!(s.saved_tokens, 239_523);
        assert!(s.saved_pct >= 99);
    }
}
