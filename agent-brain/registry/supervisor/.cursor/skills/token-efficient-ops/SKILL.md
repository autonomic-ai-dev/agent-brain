---
name: token-efficient-ops
description: Token-efficient shell and file inspection — grep, head, and bounded reads before full file loads. Use when exploring codebases, logs, or debugging without burning context on huge files.
---

# Token-efficient operations

Use this skill whenever the agent would read files, search the repo, or inspect build output.

## Search and read (in order)

1. **Grep first** — agent-brain **`grep_search`** (or `rg -n`); never `cat` / full **Read** to find a string.
2. **Bounded read** — **`file_summary`** then **`read_file_head`** / **`read_file_tail`** (default ≤200 lines).
3. **Full read only** when grep/head proved the file is small or the whole file is required.

## Never without explicit user approval

- Reading `dist/`, `build/`, `node_modules/`, `.git/`, or `target/` trees
- `cat` on logs >500 lines or minified bundles
- Dumping entire directories into context

## Shell patterns

```bash
# Good
rg -n "pattern" src/
wc -l path/to/file.rs
sed -n '1,120p' path/to/large.log

# Bad
cat path/to/large.log
find . -type f -exec cat {} \;
```

## When a mistake happens

Call **`store_memory`** with `polarity: "negative"` (max 50 words) so agent-brain surfaces it in **`must_apply`** on similar tasks.
