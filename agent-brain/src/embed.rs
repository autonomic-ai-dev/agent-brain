use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    pub fn new() -> Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(false),
        )
        .context("init fastembed")?;
        Ok(Self { model })
    }

    pub fn dim(&self) -> usize {
        self.model
            .embed(vec!["probe".to_string()], None)
            .map(|v| v.first().map(|e| e.len()).unwrap_or(384))
            .unwrap_or(384)
    }

    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        self.model
            .embed(texts.to_vec(), None)
            .context("embed texts")
    }

    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut emb = self.embed(&[text.to_string()])?.remove(0);
        l2_normalize(&mut emb);
        Ok(emb)
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
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
    }
    dot
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
}
