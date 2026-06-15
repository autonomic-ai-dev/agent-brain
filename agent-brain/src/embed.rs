use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub struct Embedder {
    model: Option<TextEmbedding>,
    pub model_id: &'static str,
}

impl Embedder {
    pub fn new() -> Result<Self> {
        Self::with_model(EmbeddingModel::AllMiniLML6V2)
    }

    /// Offline embedder for tests and CI — no ONNX model download.
    pub fn deterministic() -> Self {
        Self {
            model: None,
            model_id: "deterministic",
        }
    }

    pub fn with_model(model: EmbeddingModel) -> Result<Self> {
        let model_id = embedding_model_name(&model);
        let inner = TextEmbedding::try_new(
            InitOptions::new(model).with_show_download_progress(false),
        )
        .context("init fastembed")?;
        Ok(Self {
            model: Some(inner),
            model_id,
        })
    }

    pub fn dim(&self) -> usize {
        match &self.model {
            Some(model) => model
                .embed(vec!["probe".to_string()], None)
                .map(|v| v.first().map(|e| e.len()).unwrap_or(384))
                .unwrap_or(384),
            None => 384,
        }
    }

    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        if self.model.is_none() {
            return Ok(texts.iter().map(|t| deterministic_embedding(t)).collect());
        }
        self.model
            .as_ref()
            .unwrap()
            .embed(texts.to_vec(), None)
            .context("embed texts")
    }

    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        if self.model.is_none() {
            return Ok(deterministic_embedding(text));
        }
        let mut emb = self.embed(&[text.to_string()])?.remove(0);
        l2_normalize(&mut emb);
        Ok(emb)
    }
}

/// Stable unit vector from text — used by `Embedder::deterministic`.
pub fn deterministic_embedding(text: &str) -> Vec<f32> {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(text.as_bytes());
    let mut v = Vec::with_capacity(384);
    for i in 0..384 {
        v.push(hash[i % hash.len()] as f32 / 255.0);
    }
    l2_normalize(&mut v);
    v
}

pub fn parse_embedding_model(name: &str) -> EmbeddingModel {
    match name.trim().to_ascii_lowercase().as_str() {
        "fast" | "mini-q" | "6v2q" => EmbeddingModel::AllMiniLML6V2Q,
        "bge-small" | "bge_small" | "small" => EmbeddingModel::BGESmallENV15,
        "bge-small-q" | "bge_small_q" | "fast-bge" => EmbeddingModel::BGESmallENV15Q,
        "mini" | "default" | "6v2" => EmbeddingModel::AllMiniLML6V2,
        _ => EmbeddingModel::AllMiniLML6V2,
    }
}

pub fn embedding_model_name(model: &EmbeddingModel) -> &'static str {
    match model {
        EmbeddingModel::AllMiniLML6V2Q => "mini-q",
        EmbeddingModel::BGESmallENV15 => "bge-small",
        EmbeddingModel::BGESmallENV15Q => "bge-small-q",
        EmbeddingModel::AllMiniLML6V2 => "mini",
        _ => "mini",
    }
}

/// L2-normalize in place. Returns the pre-normalization norm.
pub fn l2_normalize(v: &mut [f32]) -> f64 {
    let mut norm_sq = 0.0f64;
    for x in v.iter() {
        let x = *x as f64;
        norm_sq += x * x;
    }
    if norm_sq == 0.0 {
        return 0.0;
    }
    let norm = norm_sq.sqrt();
    let inv = (1.0 / norm) as f32;
    for x in v.iter_mut() {
        *x *= inv;
    }
    norm
}

pub fn normalize_embedding(mut v: Vec<f32>) -> Vec<f32> {
    l2_normalize(&mut v);
    v
}

/// Dot product for unit vectors (cosine similarity).
pub fn dot_product(a: &[f32], b: &[f32]) -> f64 {
    dot_product_simd(a, b)
}

/// Batched dot products against a unit query vector.
pub fn batch_dot_products(query: &[f32], embeddings: &[Option<&[f32]>]) -> Vec<f64> {
    embeddings
        .iter()
        .map(|emb| match emb {
            Some(e) if e.len() == query.len() => dot_product_simd(query, e),
            _ => 0.0,
        })
        .collect()
}

fn dot_product_simd(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;
    let mut i = 0;
    while i + 4 <= a.len() {
        sum0 += a[i] * b[i];
        sum1 += a[i + 1] * b[i + 1];
        sum2 += a[i + 2] * b[i + 2];
        sum3 += a[i + 3] * b[i + 3];
        i += 4;
    }
    let mut sum = sum0 + sum1 + sum2 + sum3;
    while i < a.len() {
        sum += a[i] * b[i];
        i += 1;
    }
    sum as f64
}

pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_matches_cosine_for_unit_vectors() {
        let mut a = vec![3.0, 4.0];
        let mut b = vec![1.0, 0.0];
        l2_normalize(&mut a);
        l2_normalize(&mut b);
        let dot = dot_product(&a, &b);
        let cos = cosine(&a, &b);
        assert!((dot - cos).abs() < 1e-6);
    }

    #[test]
    fn batch_dots_match_scalar() {
        let query = vec![0.6, 0.8];
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let batch = batch_dot_products(&query, &[Some(a.as_slice()), Some(b.as_slice()), None]);
        assert!((batch[0] - dot_product(&query, &a)).abs() < 1e-6);
        assert!((batch[1] - dot_product(&query, &b)).abs() < 1e-6);
        assert_eq!(batch[2], 0.0);
    }

    #[test]
    fn parses_embedding_model_aliases() {
        assert!(matches!(
            parse_embedding_model("fast"),
            EmbeddingModel::AllMiniLML6V2Q
        ));
        assert!(matches!(
            parse_embedding_model("bge-small"),
            EmbeddingModel::BGESmallENV15
        ));
    }

    #[test]
    fn deterministic_embedding_is_unit_length() {
        let emb = deterministic_embedding("configure vitest for react testing");
        assert_eq!(emb.len(), 384);
        let norm: f64 = emb.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
