use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Rule,
    Skill,
    Agent,
    Memory,
    Workflow,
}

impl ItemType {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemType::Rule => "rule",
            ItemType::Skill => "skill",
            ItemType::Agent => "agent",
            ItemType::Memory => "memory",
            ItemType::Workflow => "workflow",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rule" => Some(ItemType::Rule),
            "skill" => Some(ItemType::Skill),
            "agent" => Some(ItemType::Agent),
            "memory" => Some(ItemType::Memory),
            "workflow" => Some(ItemType::Workflow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredItem {
    pub id: String,
    pub item_type: ItemType,
    pub topic: String,
    pub text: String,
    pub source_path: Option<String>,
    pub scope: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polarity: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub apply_when_matched: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteLimits {
    #[serde(default = "default_agents")]
    #[schemars(default = "default_agents")]
    pub agents: usize,
    #[serde(default = "default_skills")]
    #[schemars(default = "default_skills")]
    pub skills: usize,
    #[serde(default = "default_rules")]
    #[schemars(default = "default_rules")]
    pub rules: usize,
    #[serde(default = "default_memory")]
    #[schemars(default = "default_memory")]
    pub memory: usize,
}

impl Default for RouteLimits {
    fn default() -> Self {
        Self {
            agents: default_agents(),
            skills: default_skills(),
            rules: default_rules(),
            memory: default_memory(),
        }
    }
}

impl RouteLimits {
    /// MCP clients may send the JSON-schema zero object; treat that as unset.
    pub fn normalize(self) -> Self {
        if self.agents == 0 && self.skills == 0 && self.rules == 0 && self.memory == 0 {
            return Self::default();
        }
        self
    }
}

/// Serde helper: normalize limits after every MCP/client parse.
pub fn deserialize_route_limits<'de, D>(deserializer: D) -> Result<RouteLimits, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(RouteLimits::deserialize(deserializer)?.normalize())
}

pub fn default_agents() -> usize {
    2
}
pub fn default_skills() -> usize {
    3
}
pub fn default_rules() -> usize {
    5
}
pub fn default_memory() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRec {
    pub name: String,
    pub path: String,
    pub rationale: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRec {
    pub name: String,
    pub path: String,
    pub rationale: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleRec {
    pub topic: String,
    pub text: String,
    pub source_path: String,
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    #[default]
    Implementing,
    Verification,
    Debugging,
    Review,
    Architecture,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Implementing => "implementing",
            TaskKind::Verification => "verification",
            TaskKind::Debugging => "debugging",
            TaskKind::Review => "review",
            TaskKind::Architecture => "architecture",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "implementing" | "implementation" => Some(TaskKind::Implementing),
            "verification" | "verify" | "testing" => Some(TaskKind::Verification),
            "debugging" | "debug" => Some(TaskKind::Debugging),
            "review" | "reviewing" => Some(TaskKind::Review),
            "architecture" | "architect" | "planning" => Some(TaskKind::Architecture),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextBundle {
    pub team_rules: Vec<RuleRec>,
    pub negative_memory: Vec<MemoryRec>,
    pub skill_docs: Vec<SkillRec>,
    pub agents: Vec<AgentRec>,
    pub observations: Vec<MemoryRec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRec {
    pub topic: String,
    pub text: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MustApply {
    pub topic: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteWarning {
    pub topic: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedTool {
    pub server: String,
    pub tool: String,
    pub description: String,
    pub rationale: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedNativeTool {
    pub tool: String,
    pub description: String,
    pub rationale: String,
    pub score: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteTaskResponse {
    pub recommended_agents: Vec<AgentRec>,
    pub recommended_skills: Vec<SkillRec>,
    pub applicable_rules: Vec<RuleRec>,
    pub relevant_memory: Vec<MemoryRec>,
    pub must_apply: Vec<MustApply>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<RouteWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_tools: Vec<SuggestedTool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_native_tools: Vec<SuggestedNativeTool>,
    pub recommended_phase: String,
    pub tokens_used: usize,
    pub tokens_budget: usize,
    pub cache_hit: bool,
    pub latency_ms: u64,
    pub log_id: String,
    /// Total indexed items considered during routing (skills + memory + rules in index).
    #[serde(default)]
    pub index_total: usize,
    /// One-line human summary (full markdown: `agent-brain briefing` or logs/last-route.md)
    pub briefing: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_kind: Option<String>,
    #[serde(default)]
    pub route_confidence: f64,
    #[serde(default)]
    pub escalate_recommended: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_bundle: Option<ContextBundle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_context: Option<crate::graphify::CodeContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetContextResponse {
    pub items: Vec<GetContextItem>,
    pub tokens_used: usize,
    pub tokens_budget: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetContextItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub topic: String,
    pub text: String,
    pub score: f64,
    pub scope: String,
    pub source_path: Option<String>,
}
