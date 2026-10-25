use crate::db::store::BrainStore;
use crate::settings::UpstreamMcpSettings;
use crate::types::SuggestedTool;
use crate::upstream::{enabled_servers, IndexedUpstreamTool};
use crate::workspace::{is_mcp_host_task, mcp_route_expansion_tags};

pub fn suggest_upstream_tools(
    store: &BrainStore,
    settings: &UpstreamMcpSettings,
    user_message: &str,
    limit: usize,
) -> Vec<SuggestedTool> {
    if !settings.enabled || limit == 0 {
        return Vec::new();
    }
    let Ok(tools) = store.list_upstream_tools() else {
        return Vec::new();
    };
    if tools.is_empty() {
        return Vec::new();
    }

    let allowed: std::collections::HashSet<String> = enabled_servers(settings)
        .into_iter()
        .map(|s| s.name.to_ascii_lowercase())
        .collect();

    let mut query = user_message.to_ascii_lowercase();
    if is_mcp_host_task(user_message) {
        let expansion = mcp_route_expansion_tags(user_message).join(" ");
        query = format!("{query} {expansion}");
    }

    let query_tokens: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .collect();

    let history_adjustments = compute_historical_outcome_adjustments(store);

    let mut scored: Vec<(f64, &IndexedUpstreamTool)> = tools
        .iter()
        .filter(|tool| allowed.contains(&tool.server.to_ascii_lowercase()))
        .map(|tool| {
            let base = score_tool(&query, &query_tokens, tool);
            let hist = history_adjustments.get(&tool.name).copied().unwrap_or(0.0);
            (base + hist, tool)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let top_score = scored.first().map(|s| s.0).unwrap_or(0.0);
    let min_relative = (top_score * 0.50).max(top_score - 1.5);

    scored
        .into_iter()
        .filter(|(score, _)| *score >= min_relative)
        .take(limit)
        .map(|(score, tool)| {
            let hist_note = match history_adjustments.get(&tool.name).copied() {
                Some(adj) if adj > 0.1 => format!(" (outcome reinforced: +{:.1})", adj),
                Some(adj) if adj < -0.1 => format!(" (outcome dampened: {:.1})", adj),
                _ => String::new(),
            };
            let rationale = format!("keyword match for {}{}", tool.name, hist_note);
            SuggestedTool {
                server: tool.server.clone(),
                tool: tool.name.clone(),
                description: tool.description.clone(),
                rationale,
                score,
            }
        })
        .collect()
}

fn compute_historical_outcome_adjustments(
    store: &BrainStore,
) -> std::collections::HashMap<String, f64> {
    let mut adjustments = std::collections::HashMap::new();
    let Ok(outcomes) = store.list_verify_outcomes(50) else {
        return adjustments;
    };
    if outcomes.is_empty() {
        return adjustments;
    }

    for row in outcomes {
        let cmd_lower = row.cmd.to_ascii_lowercase();
        let delta = if row.exit_code == 0 { 0.4 } else { -0.4 };

        if cmd_lower.contains("workflow")
            || cmd_lower.contains("dag")
            || cmd_lower.contains("spine")
        {
            *adjustments
                .entry("spine_submit_workflow".to_string())
                .or_insert(0.0) += delta;
        }
    }

    for val in adjustments.values_mut() {
        *val = val.clamp(-2.0, 2.0);
    }

    adjustments
}

fn score_tool(query: &str, query_tokens: &[&str], tool: &IndexedUpstreamTool) -> f64 {
    let haystack = format!(
        "{} {} {}",
        tool.server.to_ascii_lowercase(),
        tool.name.to_ascii_lowercase(),
        tool.description.to_ascii_lowercase()
    );
    let mut score = 0.0;
    for token in query_tokens {
        if haystack.contains(token) {
            score += 1.0;
        }
    }
    if query.contains(&tool.name.to_ascii_lowercase()) {
        score += 2.0;
    }
    if query.contains(&tool.server.to_ascii_lowercase()) {
        score += 1.5;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upstream::IndexedUpstreamTool;

    #[test]
    fn scores_github_issue_lookup() {
        let tools = vec![IndexedUpstreamTool {
            server: "github".into(),
            name: "search_issues".into(),
            description: "Search GitHub issues in a repository".into(),
        }];
        let query = "find open github issues in agent-brain repo";
        let query_lower = query.to_ascii_lowercase();
        let query_tokens: Vec<&str> = query_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 3)
            .collect();
        let score = score_tool(&query_lower, &query_tokens, &tools[0]);
        assert!(score >= 2.0);
    }
}
