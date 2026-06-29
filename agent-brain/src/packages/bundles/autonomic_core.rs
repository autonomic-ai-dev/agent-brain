//! Embedded @autonomic-core bundle — find-skills and registry discovery helpers.

pub fn files() -> &'static [(&'static str, &'static str)] {
    &[
        (
            ".cursor/skills/find-skills/SKILL.md",
            include_str!(
                "../../../registry/autonomic-core/.cursor/skills/find-skills/SKILL.md"
            ),
        ),
        (
            ".cursor/skills/rmcp-mcp-gateway/SKILL.md",
            include_str!(
                "../../../registry/autonomic-core/.cursor/skills/rmcp-mcp-gateway/SKILL.md"
            ),
        ),
    ]
}
