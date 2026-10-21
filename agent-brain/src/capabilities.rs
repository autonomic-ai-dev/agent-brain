use crate::db::store::BrainStore;
use crate::settings::UpstreamMcpSettings;
use crate::types::MustApply;

pub struct McpServerCount {
    pub name: String,
    pub tools: usize,
}

pub fn capability_must_apply(servers: &[McpServerCount]) -> MustApply {
    let servers_text = if servers.is_empty() {
        "no MCP servers are configured".to_string()
    } else {
        let list = servers
            .iter()
            .map(|s| format!("{} ({} tools)", s.name, s.tools))
            .collect::<Vec<_>>()
            .join(", ");
        format!("MCP servers reachable through route_to_mcp: {list}")
    };
    MustApply {
        topic: "capability-discovery".into(),
        text: format!(
            "{servers_text}. Before saying a skill, tool, or MCP server is unavailable, check skill_search and this list; run `agent-brain mcp list` to confirm."
        ),
    }
}

pub fn capability_rule_for(store: &BrainStore, upstream: &UpstreamMcpSettings) -> MustApply {
    let indexed = store.list_upstream_tools().unwrap_or_default();
    let servers: Vec<McpServerCount> = crate::upstream::enabled_servers(upstream)
        .iter()
        .map(|server| McpServerCount {
            name: server.name.clone(),
            tools: indexed
                .iter()
                .filter(|tool| tool.server.eq_ignore_ascii_case(&server.name))
                .count(),
        })
        .collect();
    capability_must_apply(&servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_each_server_with_its_tool_count() {
        let servers = [
            McpServerCount {
                name: "linear".into(),
                tools: 68,
            },
            McpServerCount {
                name: "stitch".into(),
                tools: 16,
            },
        ];
        let rule = capability_must_apply(&servers);
        assert_eq!(rule.topic, "capability-discovery");
        assert!(rule.text.contains("linear (68 tools)"));
        assert!(rule.text.contains("stitch (16 tools)"));
    }

    #[test]
    fn still_tells_the_agent_to_check_before_claiming_unavailable_without_servers() {
        let rule = capability_must_apply(&[]);
        assert!(rule.text.contains("no MCP servers"));
        assert!(rule.text.contains("skill_search"));
    }
}
