use std::fs;
use std::path::Path;

use chrono::Local;

use crate::types::RouteTaskResponse;

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
    if !resp.log_id.is_empty() {
        out.push_str(&format!("Log: `{}`\n", resp.log_id));
    }
    out.push('\n');

    section_list(
        &mut out,
        "Agents",
        resp.recommended_agents.iter().map(|a| {
            format!(
                "**{}** — {} _(score {:.2})_",
                a.name, a.rationale, a.score
            )
        }),
    );
    section_list(
        &mut out,
        "Skills",
        resp.recommended_skills.iter().map(|s| {
            format!(
                "**{}** — {} _(score {:.2})_",
                s.name, s.rationale, s.score
            )
        }),
    );
    section_list(
        &mut out,
        "Rules",
        resp.applicable_rules.iter().map(|r| {
            let preview: String = r.text.chars().take(80).collect();
            let ellipsis = if r.text.chars().count() > 80 { "…" } else { "" };
            format!(
                "**{}** — {}{} _(score {:.2})_",
                r.topic, preview, ellipsis, r.score
            )
        }),
    );
    section_list(
        &mut out,
        "Memory",
        resp.relevant_memory.iter().map(|m| {
            format!("**{}** — {} _(score {:.2})_", m.topic, m.text, m.score)
        }),
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
    format!(
        "{} agents, {} skills, {} rules, {} memory · phase={} · {}ms · log={} · details: {}",
        resp.recommended_agents.len(),
        resp.recommended_skills.len(),
        resp.applicable_rules.len(),
        resp.relevant_memory.len(),
        resp.recommended_phase,
        resp.latency_ms,
        resp.log_id,
        briefing_path_display()
    )
}

pub fn format_stderr_line(resp: &RouteTaskResponse) -> String {
    format!("agent-brain: {}", format_summary_line(resp))
}

fn briefing_path_display() -> String {
    let home = std::env::var("AGENT_BRAIN_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".agent_brain")
        });
    home.join("logs/last-route.md").display().to_string()
}

pub fn publish_briefing(home: &Path, resp: &RouteTaskResponse, stderr_line: bool) {
    let briefing = format_briefing(resp);
    let logs = home.join("logs");
    if fs::create_dir_all(&logs).is_ok() {
        let path = logs.join("last-route.md");
        let _ = fs::write(&path, &briefing);
    }
    if stderr_line {
        eprintln!("{}", format_stderr_line(resp));
    }
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
            }],
            recommended_phase: "debugging".into(),
            tokens_used: 100,
            tokens_budget: 500,
            latency_ms: 12,
            log_id: "abc".into(),
            ..Default::default()
        };
        let text = format_briefing(&resp);
        assert!(text.contains("rust-reviewer"));
        assert!(text.contains("rust-testing"));
        assert!(text.contains("debugging"));
    }
}
