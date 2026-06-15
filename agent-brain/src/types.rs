use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Rule,
    Skill,
    Agent,
    Memory,
}

impl ItemType {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemType::Rule => "rule",
            ItemType::Skill => "skill",
            ItemType::Agent => "agent",
            ItemType::Memory => "memory",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rule" => Some(ItemType::Rule),
            "skill" => Some(ItemType::Skill),
            "agent" => Some(ItemType::Agent),
            "memory" => Some(ItemType::Memory),
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RouteLimits {
    #[serde(default = "default_agents")]
    pub agents: usize,
    #[serde(default = "default_skills")]
    pub skills: usize,
    #[serde(default = "default_rules")]
    pub rules: usize,
    #[serde(default = "default_memory")]
    pub memory: usize,
}

fn default_agents() -> usize {
    2
}
fn default_skills() -> usize {
    3
}
fn default_rules() -> usize {
    5
}
fn default_memory() -> usize {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteTaskResponse {
    pub recommended_agents: Vec<AgentRec>,
    pub recommended_skills: Vec<SkillRec>,
    pub applicable_rules: Vec<RuleRec>,
    pub relevant_memory: Vec<MemoryRec>,
    pub must_apply: Vec<MustApply>,
    pub recommended_phase: String,
    pub tokens_used: usize,
    pub tokens_budget: usize,
    pub cache_hit: bool,
    pub latency_ms: u64,
    pub log_id: String,
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
