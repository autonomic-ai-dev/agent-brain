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

    let mcp_task = is_mcp_host_task(user_message);

    let mut scored: Vec<(f64, &IndexedUpstreamTool)> = tools
        .iter()
        .filter(|tool| allowed.contains(&tool.server.to_ascii_lowercase()))
        .map(|tool| (score_tool(&query, &query_tokens, tool, mcp_task), tool))
        .filter(|(score, _)| *score > 0.0)
        .collect();

    if scored.is_empty() && mcp_task {
        scored = tools
            .iter()
            .filter(|tool| {
                allowed.contains(&tool.server.to_ascii_lowercase())
                    && tool.server.eq_ignore_ascii_case("agent-body")
            })
            .map(|tool| (default_agent_body_mcp_score(tool), tool))
            .collect();
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .take(limit)
        .map(|(score, tool)| SuggestedTool {
            server: tool.server.clone(),
            tool: tool.name.clone(),
            description: tool.description.clone(),
            rationale: if mcp_task && tool.server.eq_ignore_ascii_case("agent-body") {
                format!("MCP host task — agent-body gateway tool {}", tool.name)
            } else {
                format!("keyword match for {}", tool.name)
            },
            score,
        })
        .collect()
}

fn score_tool(
    query: &str,
    query_tokens: &[&str],
    tool: &IndexedUpstreamTool,
    mcp_task: bool,
) -> f64 {
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
    if mcp_task && tool.server.eq_ignore_ascii_case("agent-body") {
        score += 3.0;
        score += default_agent_body_mcp_score(tool) * 0.25;
    }
    score
}

fn default_agent_body_mcp_score(tool: &IndexedUpstreamTool) -> f64 {
    match tool.name.as_str() {
        "muscle_execute_bash" => 4.0,
        "spine_list_workflows" => 3.5,
        name if name.starts_with("spine_") => 3.0,
        name if name.starts_with("mouth_validate") => 2.8,
        _ => 2.5,
    }
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
        let score = score_tool(&query_lower, &query_tokens, &tools[0], false);
        assert!(score >= 2.0);
    }

    #[test]
    fn boosts_agent_body_on_mcp_host_task() {
        let tool = IndexedUpstreamTool {
            server: "agent-body".into(),
            name: "muscle_execute_bash".into(),
            description: "Execute a shell command".into(),
        };
        let query = "cursor mcp tools not registering";
        let query_lower = query.to_ascii_lowercase();
        let tokens: Vec<&str> = query_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 3)
            .collect();
        let plain = score_tool(&query_lower, &tokens, &tool, false);
        let boosted = score_tool(&query_lower, &tokens, &tool, true);
        assert!(boosted > plain);
        assert!(boosted >= 3.0);
    }

    #[test]
    fn default_agent_body_fallback_score() {
        let tool = IndexedUpstreamTool {
            server: "agent-body".into(),
            name: "spine_list_workflows".into(),
            description: "List workflows".into(),
        };
        assert!(default_agent_body_mcp_score(&tool) >= 3.0);
    }
}
