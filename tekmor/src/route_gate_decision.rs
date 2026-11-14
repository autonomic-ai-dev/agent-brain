use crate::route_gate_event::{classify_tool, ToolKind};

const STALE_ROUTE_SECS: f64 = 45.0;

const META_DISCOVERY_TOOLS: [&str; 11] = [
    "toolsearch",
    "tool_search",
    "tool-search",
    "list_tools",
    "listtools",
    "list-tools",
    "search_tools",
    "searchtools",
    "search-tools",
    "mcp_list_tools",
    "mcp_search_tools",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GateState {
    pub needs_route: bool,
    pub needs_route_since: Option<f64>,
    pub route_grace_until: Option<f64>,
    pub mcp_offline_until: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AllowWithoutRoute(GraceReason),
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraceReason {
    McpOffline,
    RouteGrace,
    StaleTimeout,
}

pub fn is_meta_or_discovery(tool: &str) -> bool {
    let lower = tool.trim().to_ascii_lowercase();
    let clean = ["mcp__", "mcp_", "mcp:"]
        .iter()
        .find_map(|prefix| lower.strip_prefix(prefix))
        .unwrap_or(&lower);
    META_DISCOVERY_TOOLS.contains(&lower.as_str()) || META_DISCOVERY_TOOLS.contains(&clean)
}

fn active_until(until: Option<f64>, now: f64) -> bool {
    until.is_some_and(|until| until > 0.0 && now < until)
}

fn stale(state: &GateState, now: f64) -> bool {
    state.needs_route
        && state
            .needs_route_since
            .is_some_and(|since| since > 0.0 && now - since >= STALE_ROUTE_SECS)
}

pub fn decide(tool: &str, state: &GateState, now: f64) -> Decision {
    if classify_tool(tool) == ToolKind::RouteTask || is_meta_or_discovery(tool) {
        return Decision::Allow;
    }
    if !state.needs_route {
        return Decision::Allow;
    }
    if active_until(state.mcp_offline_until, now) {
        Decision::AllowWithoutRoute(GraceReason::McpOffline)
    } else if active_until(state.route_grace_until, now) {
        Decision::AllowWithoutRoute(GraceReason::RouteGrace)
    } else if stale(state, now) {
        Decision::AllowWithoutRoute(GraceReason::StaleTimeout)
    } else {
        Decision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000.0;

    fn needing_route() -> GateState {
        GateState {
            needs_route: true,
            needs_route_since: Some(NOW),
            ..GateState::default()
        }
    }

    #[test]
    fn a_tool_is_denied_until_route_task_runs() {
        assert_eq!(decide("Read", &needing_route(), NOW), Decision::Deny);
        assert_eq!(decide("Bash", &needing_route(), NOW), Decision::Deny);
    }

    #[test]
    fn route_task_and_discovery_tools_are_always_allowed() {
        for tool in ["route_task", "mcp__tekmor__route_task", "ToolSearch", "mcp__list_tools"] {
            assert_eq!(decide(tool, &needing_route(), NOW), Decision::Allow, "{tool}");
        }
    }

    #[test]
    fn nothing_is_denied_once_a_route_is_current() {
        assert_eq!(decide("Read", &GateState::default(), NOW), Decision::Allow);
    }

    #[test]
    fn an_active_grace_window_allows_with_its_reason() {
        let state = GateState {
            route_grace_until: Some(NOW + 60.0),
            ..needing_route()
        };
        assert_eq!(
            decide("Read", &state, NOW),
            Decision::AllowWithoutRoute(GraceReason::RouteGrace)
        );
    }

    #[test]
    fn an_expired_grace_window_does_not_allow() {
        let state = GateState {
            route_grace_until: Some(NOW - 1.0),
            ..needing_route()
        };
        assert_eq!(decide("Read", &state, NOW), Decision::Deny);
    }

    #[test]
    fn mcp_offline_takes_precedence_over_grace() {
        let state = GateState {
            route_grace_until: Some(NOW + 60.0),
            mcp_offline_until: Some(NOW + 60.0),
            ..needing_route()
        };
        assert_eq!(
            decide("Read", &state, NOW),
            Decision::AllowWithoutRoute(GraceReason::McpOffline)
        );
    }

    #[test]
    fn a_route_pending_past_the_stale_limit_is_allowed() {
        let state = GateState {
            needs_route_since: Some(NOW - STALE_ROUTE_SECS),
            ..needing_route()
        };
        assert_eq!(
            decide("Read", &state, NOW),
            Decision::AllowWithoutRoute(GraceReason::StaleTimeout)
        );
    }

    #[test]
    fn a_route_pending_below_the_stale_limit_is_still_denied() {
        let state = GateState {
            needs_route_since: Some(NOW - STALE_ROUTE_SECS + 1.0),
            ..needing_route()
        };
        assert_eq!(decide("Read", &state, NOW), Decision::Deny);
    }
}
