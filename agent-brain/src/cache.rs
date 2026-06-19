use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lru::LruCache;

use crate::types::RouteTaskResponse;

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct CacheKey {
    pub scope_key: String,
    pub phase: String,
    pub open_files_fp: String,
    pub query_fp: String,
    pub index_version: u64,
    pub task_kind: String,
}

pub struct TurnCache {
    inner: Mutex<LruCache<CacheKey, (RouteTaskResponse, Instant)>>,
    ttl: Duration,
}

impl TurnCache {
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity.max(1)).unwrap(),
            )),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<RouteTaskResponse> {
        let mut guard = self.inner.lock().ok()?;
        let (resp, ts) = guard.get(key)?;
        if ts.elapsed() > self.ttl {
            return None;
        }
        let mut out = resp.clone();
        out.cache_hit = true;
        Some(out)
    }

    pub fn put(&self, key: CacheKey, resp: RouteTaskResponse) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.put(key, (resp, Instant::now()));
        }
    }

    pub fn remove(&self, key: &CacheKey) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.pop(key);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.clear();
        }
    }
}

pub fn route_cache_key(
    scope_key: &str,
    phase: &str,
    task_kind: &str,
    open_files: &[String],
    user_message: &str,
    index_version: u64,
    ignore_open_files: bool,
) -> CacheKey {
    CacheKey {
        scope_key: scope_key.to_string(),
        phase: phase.to_string(),
        task_kind: task_kind.to_string(),
        open_files_fp: if ignore_open_files {
            String::new()
        } else {
            fingerprint_open_files(open_files)
        },
        query_fp: fingerprint_query(user_message),
        index_version,
    }
}

pub fn fingerprint_open_files(files: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted = files.to_vec();
    sorted.sort();
    let joined = sorted.join("|");
    format!("{:x}", Sha256::digest(joined.as_bytes()))
}

pub struct QueryEmbeddingCache {
    inner: Mutex<LruCache<String, Vec<f32>>>,
}

impl QueryEmbeddingCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity.max(1)).unwrap(),
            )),
        }
    }

    pub fn get(&self, key: &str) -> Option<Vec<f32>> {
        self.inner.lock().ok()?.get(key).cloned()
    }

    pub fn put(&self, key: impl Into<String>, embedding: Vec<f32>) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.put(key.into(), embedding);
        }
    }
}

pub fn fingerprint_query(message: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized: String = message
        .chars()
        .take(128)
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}
