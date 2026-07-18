---
name: product-feature-spec
description: Constitution → feature spec → spine workflow lane. Use when implementing from docs/features/*.md and CONSTITUTION.md instead of ad hoc prompts.
---

# Product feature spec

Implement from **written intent**, not chat drift.

## Before every turn

1. Read `CONSTITUTION.md` at repo root (if present).
2. Read the active feature spec path from spine payload (`feature_spec`) or user message.
3. Call `route_task` with the user message + `current_working_directory` + open files.

## While implementing

- Map each edit to an **acceptance criterion** in the spec.
- Add or update tests listed in the spec **Test plan** before claiming done.
- Match stack and patterns from the constitution; do not introduce new deps without spec approval.
- Keep diffs minimal; no drive-by refactors.

## Verification

Run the spec test plan, then project linters. Do not skip failing tests.

## Task end

- Call `store_memory` once for durable decisions (max 50 words, no secrets).
- Update spec checkboxes if the team tracks status in git.

## Spine handoff

When `agent-spine` shows a pending Agent node:

```bash
agent-spine watch   # pending node + payload
```

Paste into Cursor: node `description` + relevant spec sections + constitution constraints.

## Workflow

```bash
agent-spine init --with feature-from-spec
agent-spine run ~/.config/agent-spine/workflows/feature-from-spec.yaml \
  --payload '{"repo_root":".","constitution":"CONSTITUTION.md","feature_spec":"docs/features/my-feature.md"}'
```

See `agent-brain/docs/product/README.md` for the full lane.
