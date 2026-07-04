//! Filterable in-memory ANN for large indexes (BM25 ∪ vector hybrid).
//!
//! Vectors are stored as **SQ8** scalar-quantized codes (1 byte/dim + per-vector
//! scale) — ~384 B/vector for 384-dim vs ~1.5 KB f32. Uses brute-force for small
//! indexes and a single-layer HNSW graph with per-filter bridge edges for larger ones.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::db::store::CachedRow;

#[derive(Debug, Clone, Copy)]
pub struct AnnSettings {
    pub enabled: bool,
    pub min_index: usize,
    pub top_k: usize,
}

impl Default for AnnSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_index: DEFAULT_ANN_MIN_INDEX,
            top_k: DEFAULT_ANN_TOP_K,
        }
    }
}

/// Minimum indexed rows before ANN candidate expansion activates.
pub const DEFAULT_ANN_MIN_INDEX: usize = 1_500;
pub const DEFAULT_ANN_TOP_K: usize = 100;

const HNSW_MIN_NODES: usize = 256;
const M: usize = 16;
const EF_SEARCH: usize = 64;

#[derive(Debug, Clone)]
pub struct AnnFilter<'a> {
    pub repo_root: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct AnnIndex {
    ids: Vec<String>,
    dims: usize,
    /// SQ8 codes: `ids.len() * dims` bytes (i8 bit-patterns stored as u8).
    codes: Vec<u8>,
    /// Per-vector dequantization scale (`value = (code as i8) as f32 * scale`).
    scales: Vec<f32>,
    filter_buckets: Vec<u32>,
    scopes: Vec<String>,
    scope_keys: Vec<Option<String>>,
    graph: Vec<Vec<usize>>,
    bridge_edges: Vec<Vec<usize>>,
    entry: usize,
}

impl AnnIndex {
    pub fn from_rows(rows: &[CachedRow]) -> Option<Self> {
        let mut ids = Vec::new();
        let mut codes = Vec::new();
        let mut scales = Vec::new();
        let mut filter_buckets = Vec::new();
        let mut scopes = Vec::new();
        let mut scope_keys = Vec::new();
        let mut dims = 0usize;
        for row in rows {
            let Some(emb) = row.embedding.as_ref() else {
                continue;
            };
            if dims == 0 {
                dims = emb.len();
            }
            if emb.len() != dims || dims == 0 {
                continue;
            }
            let (row_codes, scale) = quantize_sq8(emb);
            ids.push(row.id.clone());
            scopes.push(row.scope.clone());
            scope_keys.push(row.scope_key.clone());
            filter_buckets.push(filter_bucket(&row.scope, row.scope_key.as_deref()));
            codes.extend_from_slice(&row_codes);
            scales.push(scale);
        }
        if ids.is_empty() {
            return None;
        }

        let (graph, bridge_edges, entry) = if ids.len() >= HNSW_MIN_NODES {
            build_filterable_hnsw(&codes, &scales, dims, &filter_buckets)
        } else {
            (Vec::new(), Vec::new(), 0)
        };

        Some(Self {
            ids,
            dims,
            codes,
            scales,
            filter_buckets,
            scopes,
            scope_keys,
            graph,
            bridge_edges,
            entry,
        })
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_active(&self, min_index: usize) -> bool {
        self.len() >= min_index
    }

    pub fn top_k(&self, query: &[f32], k: usize) -> Vec<(String, f64)> {
        self.top_k_filtered(query, k, None)
    }

    /// Top-K cosine similarity with optional scope/repo filter (filterable HNSW when large).
    pub fn top_k_filtered(
        &self,
        query: &[f32],
        k: usize,
        filter: Option<&AnnFilter<'_>>,
    ) -> Vec<(String, f64)> {
        if query.len() != self.dims || self.ids.is_empty() {
            return Vec::new();
        }
        let take = k.min(self.ids.len());
        if self.graph.is_empty() {
            return self.brute_force_top_k(query, take, filter);
        }
        self.hnsw_top_k(query, take, filter)
    }

    fn brute_force_top_k(
        &self,
        query: &[f32],
        k: usize,
        filter: Option<&AnnFilter<'_>>,
    ) -> Vec<(String, f64)> {
        let mut scores: Vec<(usize, f64)> = (0..self.ids.len())
            .filter(|&i| node_matches_filter(i, &self.scopes, &self.scope_keys, filter))
            .map(|i| (i, self.similarity(query, i)))
            .collect();
        if scores.is_empty() {
            return Vec::new();
        }
        let take = k.min(scores.len());
        scores.select_nth_unstable_by(take - 1, |a, b| b.1.partial_cmp(&a.1).unwrap());
        scores.truncate(take);
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
            .into_iter()
            .map(|(i, score)| (self.ids[i].clone(), score))
            .collect()
    }

    fn hnsw_top_k(
        &self,
        query: &[f32],
        k: usize,
        filter: Option<&AnnFilter<'_>>,
    ) -> Vec<(String, f64)> {
        let mut visited = HashSet::new();
        let mut candidates: Vec<(usize, f64)> = Vec::new();
        let mut frontier = vec![self.entry];
        visited.insert(self.entry);

        for _ in 0..EF_SEARCH {
            let mut next_frontier = Vec::new();
            for &node in &frontier {
                let score = self.similarity(query, node);
                if node_matches_filter(node, &self.scopes, &self.scope_keys, filter) {
                    candidates.push((node, score));
                }
                for nb in self.neighbors_of(node) {
                    if visited.insert(nb) {
                        next_frontier.push(nb);
                    }
                }
            }
            if next_frontier.is_empty() {
                break;
            }
            next_frontier.sort_by(|&a, &b| {
                self.similarity(query, b)
                    .partial_cmp(&self.similarity(query, a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            next_frontier.truncate(M);
            frontier = next_frontier;
        }

        if candidates.len() < k {
            for i in 0..self.ids.len() {
                if node_matches_filter(i, &self.scopes, &self.scope_keys, filter) {
                    candidates.push((i, self.similarity(query, i)));
                }
            }
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.dedup_by_key(|(i, _)| *i);
        candidates.truncate(k);
        candidates
            .into_iter()
            .map(|(i, score)| (self.ids[i].clone(), score))
            .collect()
    }

    fn neighbors_of(&self, node: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.graph.get(node).cloned().unwrap_or_default();
        if let Some(bridge) = self.bridge_edges.get(node) {
            for nb in bridge {
                if !out.contains(nb) {
                    out.push(*nb);
                }
            }
        }
        out
    }

    fn similarity(&self, query: &[f32], node: usize) -> f64 {
        let start = node * self.dims;
        sq8_dot_query(query, &self.codes[start..start + self.dims], self.scales[node])
    }

    /// Bytes used by SQ8 codes (excludes scales/graph metadata).
    pub fn codes_bytes(&self) -> usize {
        self.codes.len()
    }
}

/// Quantize a vector to SQ8: `code = round(v / scale)` clamped to i8, scale = max|v|/127.
pub fn quantize_sq8(emb: &[f32]) -> (Vec<u8>, f32) {
    let max_abs = emb
        .iter()
        .map(|v| v.abs())
        .fold(0.0f32, f32::max)
        .max(1e-12);
    let scale = max_abs / 127.0;
    let codes = emb
        .iter()
        .map(|v| {
            let q = (*v / scale).round().clamp(-127.0, 127.0) as i8;
            q as u8
        })
        .collect();
    (codes, scale)
}

/// Asymmetric SQ8 dot: query stays f32, corpus is SQ8.
pub fn sq8_dot_query(query: &[f32], codes: &[u8], scale: f32) -> f64 {
    let n = query.len().min(codes.len());
    let mut sum = 0.0f64;
    for i in 0..n {
        let v = (codes[i] as i8) as f32 * scale;
        sum += query[i] as f64 * v as f64;
    }
    sum
}

fn sq8_sq8_dot(a: &[u8], scale_a: f32, b: &[u8], scale_b: f32) -> f64 {
    let n = a.len().min(b.len());
    let mut sum = 0i32;
    for i in 0..n {
        sum += (a[i] as i8 as i32) * (b[i] as i8 as i32);
    }
    sum as f64 * scale_a as f64 * scale_b as f64
}

pub fn filter_bucket(scope: &str, scope_key: Option<&str>) -> u32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    scope.hash(&mut hasher);
    if let Some(key) = scope_key {
        key.hash(&mut hasher);
    }
    hasher.finish() as u32
}

fn node_matches_filter(
    idx: usize,
    scopes: &[String],
    scope_keys: &[Option<String>],
    filter: Option<&AnnFilter<'_>>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let scope = scopes.get(idx).map(|s| s.as_str()).unwrap_or("global");
    if scope == "global" {
        return true;
    }
    let Some(root) = filter.repo_root else {
        return true;
    };
    scope_keys
        .get(idx)
        .and_then(|k| k.as_deref())
        .map(|k| k == root)
        .unwrap_or(false)
}

fn build_filterable_hnsw(
    codes: &[u8],
    scales: &[f32],
    dims: usize,
    filter_buckets: &[u32],
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>, usize) {
    let n = codes.len() / dims;
    let mut graph = vec![Vec::new(); n];
    let mut entry = 0usize;
    let mut best = f64::NEG_INFINITY;

    for i in 0..n {
        let mut dists: Vec<(usize, f64)> = (0..i)
            .map(|j| (j, vector_sim(codes, scales, dims, i, j)))
            .collect();
        dists.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(nb, _) in dists.iter().take(M) {
            connect(&mut graph, i, nb);
        }
        let score = dists.first().map(|(_, s)| *s).unwrap_or(0.0);
        if score > best {
            best = score;
            entry = i;
        }
    }

    let mut bucket_members: HashMap<u32, Vec<usize>> = HashMap::new();
    for (idx, bucket) in filter_buckets.iter().enumerate() {
        bucket_members.entry(*bucket).or_default().push(idx);
    }

    let mut bridge_edges = vec![Vec::new(); n];
    for members in bucket_members.values() {
        if members.len() < 2 {
            continue;
        }
        for &node in members {
            let mut best_nb = None;
            let mut best_sim = f64::NEG_INFINITY;
            for &other in members {
                if node == other {
                    continue;
                }
                let sim = vector_sim(codes, scales, dims, node, other);
                if sim > best_sim {
                    best_sim = sim;
                    best_nb = Some(other);
                }
            }
            if let Some(nb) = best_nb {
                if !graph[node].contains(&nb) {
                    bridge_edges[node].push(nb);
                }
            }
        }
    }

    (graph, bridge_edges, entry)
}

fn connect(graph: &mut [Vec<usize>], a: usize, b: usize) {
    if !graph[a].contains(&b) {
        graph[a].push(b);
    }
    if !graph[b].contains(&a) {
        graph[b].push(a);
    }
}

fn vector_sim(codes: &[u8], scales: &[f32], dims: usize, a: usize, b: usize) -> f64 {
    let sa = a * dims;
    let sb = b * dims;
    sq8_sq8_dot(
        &codes[sa..sa + dims],
        scales[a],
        &codes[sb..sb + dims],
        scales[b],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn row(id: &str, emb: Vec<f32>) -> CachedRow {
        CachedRow::test_with_embedding(id, emb)
    }

    fn row_scoped(id: &str, emb: Vec<f32>, scope: &str, scope_key: Option<&str>) -> CachedRow {
        CachedRow::test_with_embedding_scoped(id, emb, scope, scope_key)
    }

    #[test]
    fn top_k_orders_by_similarity() {
        let rows = vec![
            row("a", vec![1.0, 0.0, 0.0]),
            row("b", vec![0.0, 1.0, 0.0]),
            row("c", vec![0.9, 0.1, 0.0]),
        ];
        let ann = AnnIndex::from_rows(&rows).unwrap();
        let top = ann.top_k(&[1.0, 0.0, 0.0], 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "a");
        assert_eq!(top[1].0, "c");
    }

    #[test]
    fn filterable_search_respects_scope() {
        let rows = vec![
            row_scoped("global", vec![1.0, 0.0], "global", None),
            row_scoped("proj", vec![0.99, 0.01], "project", Some("/repo")),
            row_scoped("other", vec![0.98, 0.02], "project", Some("/other")),
        ];
        let ann = AnnIndex::from_rows(&rows).unwrap();
        let filter = AnnFilter {
            repo_root: Some("/repo"),
        };
        let top = ann.top_k_filtered(&[1.0, 0.0], 3, Some(&filter));
        let ids: HashSet<_> = top.into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains("global"));
        assert!(ids.contains("proj"));
        assert!(!ids.contains("other"));
    }

    #[test]
    fn filterable_hnsw_p95_under_50ms_on_synthetic_index() {
        let dims = 32usize;
        let n = 2_000usize;
        let rows: Vec<CachedRow> = (0..n)
            .map(|i| {
                let mut emb = vec![0.0f32; dims];
                emb[i % dims] = 1.0;
                emb[(i + 1) % dims] = 0.1;
                let scope = if i % 3 == 0 { "project" } else { "global" };
                let key = if scope == "project" {
                    Some("/bench")
                } else {
                    None
                };
                row_scoped(&format!("id-{i}"), emb, scope, key)
            })
            .collect();
        let ann = AnnIndex::from_rows(&rows).unwrap();
        assert!(!ann.graph.is_empty());
        let query = {
            let mut q = vec![0.0f32; dims];
            q[0] = 1.0;
            q
        };
        let filter = AnnFilter {
            repo_root: Some("/bench"),
        };
        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = Instant::now();
            let _ = ann.top_k_filtered(&query, 50, Some(&filter));
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = samples[((samples.len() as f64 * 0.95) as usize).saturating_sub(1)];
        assert!(
            p95 <= 50.0,
            "filterable HNSW p95 {p95:.2}ms exceeds 50ms target"
        );
    }

    #[test]
    fn sq8_uses_one_byte_per_dim() {
        let dims = 384usize;
        let rows: Vec<CachedRow> = (0..10)
            .map(|i| {
                let mut emb = vec![0.0f32; dims];
                emb[i % dims] = 1.0;
                row(&format!("v{i}"), emb)
            })
            .collect();
        let ann = AnnIndex::from_rows(&rows).unwrap();
        assert_eq!(ann.codes_bytes(), 10 * dims);
        // f32 would be 4x; SQ8 must stay at 1 byte/dim.
        assert!(ann.codes_bytes() < 10 * dims * 4);
        let top = ann.top_k(&{
            let mut q = vec![0.0f32; dims];
            q[0] = 1.0;
            q
        }, 1);
        assert_eq!(top[0].0, "v0");
    }
}
