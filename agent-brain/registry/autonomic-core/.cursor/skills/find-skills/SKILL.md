---
name: find-skills
description: Helps users discover and install agent skills and registry workflows when they ask "how do I do X", "find a skill for X", "is there a skill that can...", or want to extend capabilities.
---

# Find Skills

Discover and install capabilities from the **Autonomic Registry** and the open skills ecosystem.

## When to Use This Skill

Use when the user:

- Asks "how do I do X" where X might have an existing skill or workflow
- Says "find a skill for X" or "is there a skill for X"
- Wants to search tools, templates, or workflows
- Asks about release notes, stacked PRs, or repeatable multi-step tasks

## Step 1 — Check the Autonomic Registry

```bash
agent-brain registry list
agent-brain registry list --kind skill_package
agent-brain registry list --kind workflow
```

Common aliases:

| Alias | Kind | Install |
|-------|------|---------|
| `@official` | skill packages | `agent-brain add @official` |
| `@claude-skills` | skill packages | `agent-brain add @claude-skills` |
| `@release-notes` | workflow | `agent-spine init --with @release-notes` |
| `@stacked-pr` | workflow | `agent-spine init --with @stacked-pr` |
| `@bugfix` | workflow | `agent-spine init --with @bugfix` |

`@official` includes: `github/awesome-copilot`, `anthropics/skills`, `microsoft/azure-skills`.

## Step 2 — Skills CLI and skills.sh

```bash
npx skills find [query]
npx skills add <owner/repo@skill> -g -y
```

Browse: https://skills.sh/

Prefer skills with 1K+ installs and trusted sources (`vercel-labs`, `anthropics`, `microsoft`, `github`).

## Step 3 — Present and Install

Show name, what it does, install count/source, and the exact install command. Offer to run install when the user agrees.

## Step 4 — Workflows

For multi-step tasks (release notes, rebases, bugfix loops), prefer a registry workflow over improvising:

```bash
agent-spine init --with @release-notes
agent-spine run --meta "generate release notes for v0.18.0"
```

## If Nothing Matches

Offer direct help, then suggest `npx skills init my-skill` for a custom skill.
