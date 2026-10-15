# Coding-agent rules bundle

This bundle contains the same strict engineering policy in each tool's native instruction-file format.

## Files and installation

- **Cursor:** copy `cursor/.cursor/rules/coding-policy.mdc` into your repository at `.cursor/rules/coding-policy.mdc`. The `alwaysApply: true` frontmatter makes it apply to every Cursor Agent conversation in that project.
- **Augment Code:** copy `augment/AGENTS.md` to the repository root as `AGENTS.md`, where supported by your Augment setup. If your team uses Augment's managed Rules configuration instead, paste the policy body into that configuration.
- **Claude Code:** copy `claude/CLAUDE.md` to the repository root as `CLAUDE.md`. For personal global defaults, place the contents in `~/.claude/CLAUDE.md` instead.
- **OpenCode:** copy `opencode/AGENTS.md` to the repository root as `AGENTS.md`. For personal global defaults, use `~/.config/opencode/AGENTS.md`.

## Important
- The Augment and OpenCode files share the filename `AGENTS.md`, so use the relevant version in each tool's folder when copying; do not copy both over one another.
- If you want one policy to apply across multiple agents in a single repository, consider keeping a canonical policy document and configuring each tool to include it. Do not create competing files with conflicting instructions.
- These files are guidance for the coding agent; they do not enforce coverage or quality gates by themselves. Enforce important requirements in CI, test tooling, branch protection, and SonarQube where applicable.
- The 90% coverage target applies to changed, measurable code where meaningful. It is not a reason to add shallow tests or distort metrics.
