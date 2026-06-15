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
        Ok(self.embed(&[text.to_string()])?.remove(0))
    }
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
