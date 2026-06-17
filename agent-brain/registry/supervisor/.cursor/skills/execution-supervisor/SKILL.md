---
name: execution-supervisor
description: Execution supervisor discipline — obey route_task must_apply constraints, avoid token rabbit holes, and persist anti-patterns with store_memory. Use on every agent turn in Cursor with agent-brain hooks.
---

# Execution supervisor

agent-brain is the **execution supervisor**: it routes minimal context and returns **`must_apply`** constraints you must follow before using tools.

## Every turn

1. **`route_task`** first (hooks enforce this).
2. Read **`must_apply`** — hard constraints (negative memory, apply_when rules).
3. Load only **`recommended_skills`** paths returned — do not grep the skill library.
4. Plan with token budget in mind: smallest tool output that answers the question.
5. Use agent-brain **`grep_search`**, **`file_summary`**, **`read_file_head`**, **`read_file_tail`** before Cursor Read/cat on large files.

## Persist anti-patterns

When the agent or user identifies a token trap:

```text
store_memory(
  topic: "no-read-dist",
  fact: "Never read dist/ or build output; use rg on src/ only.",
  polarity: "negative"
)
```

Future routes promote this to **`must_apply`**.

## Rabbit-hole recovery

If you already burned tokens on a bad path: stop, summarize findings in ≤5 bullets, **`store_memory`** the lesson, ask user before continuing broad reads.
