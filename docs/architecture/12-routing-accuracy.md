# 12 — Routing accuracy as the product USP

agent-brain's core value proposition is **intelligent routing**: given a task and available context, pick the right skills, agents, rules, and memory — not everything, and not the wrong thing.

If routing is wrong often, users stop calling `route_task` and the MCP becomes noise. Accuracy is therefore a **product requirement**, not a nice-to-have.

## What "accuracy" means here

For each user turn we produce ranked recommendations per type:

| Type | Success criterion |
|------|-------------------|
| **Skills** | Top-3 includes the skill whose activation text matches task intent |
| **Agents** | Top-2 includes the specialist agent for the phase/task |
| **Rules** | Applicable project rules surface before generic noise |
| **Memory** | Top-3 includes the fact that would change implementation choices |

We measure **Recall@3**: for a golden query, does the expected item appear in the top three for that type?

CI gate: **Recall@3 ≥ 0.85** for both the memory suite and the skill suite (`agent-brain proofs --ci` on an **isolated fixture**).

**Proven in CI** — see [13-proofs-and-benchmarks.md](13-proofs-and-benchmarks.md) and [`docs/benchmarks/latest.json`](../benchmarks/latest.json).

`eval --ci --live` runs against your production `brain.db` (informational; not a CI gate).

## How retrieval works (accuracy stack)

Routing is hybrid, not embedding-only:

```
user_message + workspace tags
        │
        ▼
   query embedding (always for route_task)
        │
        ▼
   BM25 prefilter (strict AND → loose OR fallback)
        │
        ▼
   hybrid score per candidate
   0.55 cosine + 0.25 BM25 + 0.20 lexical overlap
   (+ skill/agent lexical boost when overlap ≥ 0.2)
        │
        ▼
   minimum score threshold (drop weak recommendations)
        │
        ▼
   per-type limits + token budget → RouteTaskResponse
```

Key modules:

| Module | Role in accuracy |
|--------|------------------|
| `retrieval.rs` | Stopwords, synonym groups, lexical overlap, FTS query shaping |
| `index.rs` | Skill frontmatter + "When to activate" section in index text |
| `store.rs` | Hybrid scoring, BM25 fallback, memory pool cap |
| `engine.rs` | Always embed; filter below `minimum_recommendation_score` |
| `eval.rs` | Golden memory + skill suites, dual CI gate |

### Why not embedding-only?

Embeddings alone miss exact terms ("PR", "PgBouncer", "Vitest") and over-rank semantically nearby but wrong skills. BM25 + lexical overlap anchors intent keywords; embeddings handle paraphrase.

### Why not query-specific hacks?

One-off boosts for individual user phrases do not generalize and rot quickly. Improvements belong in:

- **Index text** (what we embed and search)
- **General synonym groups** (PR ↔ pull request, test ↔ pytest)
- **Scoring weights** validated by eval
- **Golden cases** that represent real task families

## Index quality drives accuracy

Skills are only as routable as their indexed text. We extract:

1. YAML `name` and `description` from frontmatter
2. **"When to activate / use"** section (primary routing signal)
3. First ~20 lines of body as fallback

Authors should write activation sections with the phrases users actually type ("review PR", "deploy with rollback", "debug failing test").

## Eval discipline

### Golden suites (`eval.rs`)

- **Memory golden** — 4 cases covering framework choice, infra, language, MCP
- **Skill golden** — 6 cases covering review, testing, DB, MCP, deploy, debug
- **Decoy skills** — unrelated topics (cooking, brand voice, investor outreach) seeded alongside correct skills

Each case uses deterministic embeddings in tests; CI uses the real embedder for queries against seeded items.

### When to extend golden cases

Add a case when:

- A real user query routed incorrectly in production
- A new item type or scoring change could regress a task family
- A synonym group or index rule is added (prove it helps generally)

Do **not** add a golden case that only passes because of a query-specific boost.

### Operator feedback loop

`report_context_useful` logs whether retrieved items helped. Over time this informs:

- Promoting high-signal memory (`promote`)
- GC of low-signal memory (`memory gc`)
- New golden cases and synonym groups

## Multi-host accuracy vs enforcement

Routing accuracy is independent of **enforcement** (Cursor hooks vs OpenCode instruction files). A host that skips `route_task` never benefits from good retrieval — see [07-enforcement-and-multi-host.md](07-enforcement-and-multi-host.md).

## Trade-offs

| Choice | Accuracy benefit | Cost |
|--------|------------------|------|
| Always embed on `route_task` | Better paraphrase matching | ~tens of ms per turn |
| Strict FTS first | Precision on multi-term queries | May need loose fallback |
| Minimum score threshold | Fewer wrong recommendations | May return empty skill slot on vague queries |
| Small memory candidate pool | Less memory crowding out skills | May miss edge-case facts |

## Alternatives considered

| Alternative | Why not primary |
|-------------|-----------------|
| LLM reranker per turn | Latency, cost, non-deterministic CI |
| Single global top-k | Memory drowns skills; types need separate budgets |
| BM25-only fast path | Regressed skill routing on real queries |
| Larger embedding model | Heavier install; hybrid already closes most gaps |

## For senior engineers and principal architects

### Accuracy is the adoption gate

Users forgive **slow** routing more than **wrong** routing. A fast wrong skill wastes the entire agent turn plus user trust. Product priority order:

1. **Correct top-3** (Recall@3 on representative goldens)
2. **Explainable enough** (path + rationale + score)
3. **Fast warm path** (cache + embed reuse)
4. **Cold install experience** (model download, prewarm)

### Measuring accuracy without fooling ourselves

| Anti-pattern | Why it fails |
|--------------|--------------|
| Boost one failing query | Does not generalize; rots |
| Golden set too small | Overfits to 4–6 phrases |
| No decoy skills | Router wins by returning anything |
| Skip skill suite | Memory-only green CI hides USP regression |

Decoy skills in `eval.rs` exist because production libraries have **many plausible-sounding** skills — accuracy is discrimination, not retrieval of any match.

### Organizational playbook

1. **Onboard team skill pack** → add 3–5 golden cases for your task families
2. **After skill rename** → run `index` + `eval --ci`
3. **Monthly** → `digest --weekly` + spot-check misroutes from user reports
4. **On misroute report** → fix index text or synonym group, not query-specific score hack

### When to invest in rerankers

Consider cross-encoder or LLM rerank when:

- Index > ~5k skills **and** hybrid Recall@3 plateaus below target
- p95 budget allows +100–300 ms
- You have budget for non-deterministic CI or offline eval only

Until then, **index quality + hybrid + eval** is higher ROI.

### Questions a PE should ask

1. Do we have **golden tasks** representing how developers actually phrase work?
2. Is **0.85** the right bar for our library, or do we need 0.95 on critical paths (security, deploy)?
3. Who owns **false-positive** reports (wrong skill shown) vs **false-negative** (empty slot)?
4. How do we tie **`report_context_useful`** to skill authoring feedback?

## Related reading

- [04-turn-routing-and-retrieval.md](04-turn-routing-and-retrieval.md) — pipeline detail
- [06-indexing-and-packages.md](06-indexing-and-packages.md) — what gets indexed
- [09-upstream-and-operator-loop.md](09-upstream-and-operator-loop.md) — eval in operator loop
