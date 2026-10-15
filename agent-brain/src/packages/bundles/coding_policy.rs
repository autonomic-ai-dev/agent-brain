//! Embedded @coding-policy bundle — strict engineering policy for coding agents.

pub fn files() -> &'static [(&'static str, &'static str)] {
    &[
        (
            ".cursor/rules/coding-policy.mdc",
            include_str!("../../../registry/coding-policy/.cursor/rules/coding-policy.mdc"),
        ),
        (
            "claude/CLAUDE.md",
            include_str!("../../../registry/coding-policy/claude/CLAUDE.md"),
        ),
        (
            "opencode/AGENTS.md",
            include_str!("../../../registry/coding-policy/opencode/AGENTS.md"),
        ),
        (
            "augment/AGENTS.md",
            include_str!("../../../registry/coding-policy/augment/AGENTS.md"),
        ),
        (
            "README.md",
            include_str!("../../../registry/coding-policy/README.md"),
        ),
    ]
}
