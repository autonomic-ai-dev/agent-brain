//! Embedded @supervisor bundle files (paths relative to package install root).

pub fn files() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "skills/token-efficient-ops/SKILL.md",
            include_str!(
                "../../../registry/supervisor/.cursor/skills/token-efficient-ops/SKILL.md"
            ),
        ),
        (
            "skills/execution-supervisor/SKILL.md",
            include_str!(
                "../../../registry/supervisor/.cursor/skills/execution-supervisor/SKILL.md"
            ),
        ),
        (
            "rules/execution-supervisor.mdc",
            include_str!("../../../registry/supervisor/.cursor/rules/execution-supervisor.mdc"),
        ),
    ]
}
