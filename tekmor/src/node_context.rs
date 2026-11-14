use crate::engine::Engine;
use crate::types::{GetContextResponse, ItemType};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeContextParams {
    pub node_kind: String,
    pub node_name: String,
    pub node_description: String,
    pub workflow_name: String,
    pub task_description: String,
}

/// Query brain for skills/rules/agents relevant to a specific workflow node.
pub fn get_context_for_node(
    engine: &Engine,
    params: &NodeContextParams,
    max_tokens: usize,
) -> Result<GetContextResponse> {
    let query = format!(
        "Workflow '{}' node '{}' (kind={}): {}. Task: {}",
        params.workflow_name,
        params.node_name,
        params.node_kind,
        params.node_description,
        params.task_description,
    );

    let include_types = &[ItemType::Rule, ItemType::Skill, ItemType::Agent];

    engine.get_context(&query, None, max_tokens, include_types)
}
