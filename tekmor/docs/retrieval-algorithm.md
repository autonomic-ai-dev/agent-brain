# Retrieval algorithm

This describes how tekmor chooses what to return for `route_task` (suggestions) and for memory search (`tekmor recall`, the `recall` MCP path). Every number below comes from the source named next to it. If you change a constant, update this page in the same change.

## 1. Route suggestions (`route_task`)

### 1.1 Candidate generation

The query goes to two sources. Their rankings are fused with reciprocal rank fusion (RRF) only to decide which rows get scored.

- **Lexical (BM25 over FTS5).** Items use `BM25_ITEMS_TOP = 150` and facts use `BM25_FACTS_TOP = 40` (`tekmor/src/db/store.rs:43-44`). The strict query ANDs the expanded terms. If it returns fewer than 5 hits, a loose query ORs them and the hits are merged (`bm25_search_items`, `store.rs:553`).
- **Vector (HNSW ANN).** Used only when the index holds at least `DEFAULT_ANN_MIN_INDEX = 1500` rows. It returns `DEFAULT_ANN_TOP_K = 100` nearest neighbours, filtered by repo root (`tekmor/src/ann.rs:30-31`).
- **RRF.** `score(d) = Σ 1 / (k + rank)` with `k = 60` (`tekmor/crates/tekmor-core/src/retrieval_fusion.rs`). Its output picks candidates. It does not set the final score.
- **Embeddings.** `all-MiniLM-L6-v2` by default (`tekmor/src/embed.rs:21`).

### 1.2 Base score

For each candidate (`score_items_with_bm25`, `tekmor/src/db/store.rs:3063-3065`):

```
score = 0.50 * cosine + 0.22 * bm25_norm + 0.18 * lexical + 0.10 * entity
```

When the call is lexical-only (`bm25_only`), the weights are `0.60 * bm25_norm + 0.25 * lexical + 0.15 * entity`.

- `cosine` is the dot product of the query embedding with the row embedding (`batch_dot_products`, `tekmor/src/embed.rs:233`). It is 0 when either embedding is missing or the length differs.
- `bm25_norm = |bm25| / max |bm25|` over this query's hits. Each FTS5 score is negative, so the absolute value is used.
- `lexical` is the fraction of synonym-expanded query terms that occur in `topic + text`. Terms are lowercased, stopwords are removed, and 22 synonym groups are applied (`tekmor/src/retrieval.rs:128`, `SYNONYM_GROUPS`).
- `entity` is the same fraction, over CamelCase, snake_case and ALLCAPS tokens of the query (`tekmor/src/retrieval.rs:205`).

### 1.3 Adjustments, in code order

Each rule applies to the running score (`store.rs:3075-3200`).

| Rule | Effect |
|---|---|
| Exact symbol match on the topic | `× 1.5` (`AST_SYMBOL_BOOST`) |
| Skill or agent with `lexical >= 0.2` | `+ 0.10·lexical + 0.08·topic_lexical` |
| Supervisor skills (`token-efficient-ops`, `execution-supervisor`) on MCP-host tasks | `+ 0.18` |
| Package path boost for skills, rules, agents | `+ pkg_boost` (`packages::path_recommend_boost`) |
| No BM25 hit and `lexical < 0.12` | `× 0.5` |
| Scope matches the repo root | `+ 0.10` |
| Project rule for this repo | `+ 0.20` |
| Global rule under `/references/` in a skills path | `- 0.15` |
| Procedural item (skill or agent) without query affinity | `× 0.2` |
| Tag appears in topic or text | `+ 0.15` (procedural items only with affinity) |
| `boost_agents` and an agent item | `+ 0.10` |
| Task phase matches the topic | `+ phase_match_boost` |
| Context weight (from learned route outcomes) | `× weight` |

Memory rows get their own rules:

| Rule | Effect |
|---|---|
| `source = user` or `confidence >= 0.95` | `+ 0.08` |
| `source = observation` | `+ 0.10` |
| `polarity = negative` | `+ 0.12` |
| Low-signal source (digests, `obs/*`, auto-route) | `- 0.35` (`memory_source_score_adjustment`) |
| `apply_when` conditions all match the context | `+ 0.15` |
| `apply_when` conditions present and not matched | `× 0.85` |
| Feedback (`feedback_multiplier`) | `× 0.5` if useless ≥ 3 and useless − useful ≥ 2; `× 1.15` if useful ≥ 3 and useless = 0; else `× 1.0` |
| Age (`ebbinghaus_multiplier`) | `× exp(−elapsed_days / stability)` |

Ebbinghaus decay (`tekmor/crates/tekmor-core/src/memory_decay.rs:7`):

```
stability_days = 30 + 60 * confidence + 15 * useful_count
multiplier     = exp(-elapsed_days / stability_days)
```

A row with no recorded age keeps multiplier 1.0.

### 1.4 Thresholds and selection

- **Minimum score.** `min = max(0.12, 0.45 × top non-memory score)` (`tekmor/src/retrieval.rs:550`). It is 0.12 when nothing non-memory scored.
- **Per-type floors** (`tekmor/src/engine.rs`, `calc_floor`). Skills: 0.48 if the top skill score is ≥ 0.70, 0.42 if ≥ 0.55, else 0.38. Agents, rules and workflows: 0.42 if the top is ≥ 0.60, 0.38 if ≥ 0.45, else 0.35. Each floor is at least `min`.
- **Memory floor.** `max(0.55 × top, 0.20)` when the top memory score is ≥ 0.50. Otherwise `max(0.45 × top, 0.15)`, or 0.15 when no memory scored.
- **Must-apply items.** `collect_must_apply_from_scored` takes at most 3 items with `apply_when` or negative polarity (`tekmor/src/engine.rs`).
- **Token budget.** Items are added in score order until the budget runs out. 120 tokens are reserved for memory when `limits.memory > 0`. Items left out are counted and reported in the `budget` warning.

### 1.5 Learned feedback

Routes record which items were used and which were not. Memory rows use their counts in `feedback_multiplier`. Every item has a context weight, which the route multiplies into its score (`score *= weight`). A used item gains 0.05 and an unused one loses 0.05, clamped to `[0.5, 1.5]`, starting from 1.0 (`tekmor/src/db/store.rs:2796-2812`).

## 2. Memory search

Memory search has two modes. Neither uses the route scoring above.

### 2.1 Keyword mode (default)

`tekmor recall <query>` calls `rank` (`tekmor/src/recall.rs`; caller in `tekmor/src/main.rs:1292`).

- Tokens are split on non-alphanumeric characters, lowercased, and kept when they have at least 2 characters.
- `score = |query tokens found in the fact| / |query tokens|`.
- Facts with score 0 are dropped. Results are sorted by score, then cut to the limit.
- There is no stemming, synonym expansion or BM25 here.
- With no query, results come in recency order and carry no score.

### 2.2 Semantic mode (`--semantic`)

`rank_semantic` computes cosine similarity between the query embedding and each fact embedding.

- Rows without an embedding are dropped.
- `--min-score` sets a floor. It requires `--semantic`.
- Results are sorted by cosine and cut to the limit.

Memory writes must be a lowercase slug topic and a one-line fact of at least 10 words (`validate_memory_text`, `tekmor/src/db/write_handler.rs`). This is why the stored text is useful on its own.

## 3. Known limits

- Scores are hand-tuned weights, not learned. The ratios are checked by unit tests (for example `recall.rs` `query_drops_facts_with_no_shared_token_and_orders_by_score`), not calibrated against a labelled set.
- Keyword recall has no synonyms, so it misses paraphrases that semantic mode would find.
- Retrieval quality is only measured on small golden sets (the recall eval). The golden queries are close paraphrases of their facts, so they cannot separate retrieval changes well.
