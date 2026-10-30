use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::process::output_with_timeout;

static QUERY_CACHE: Mutex<Option<QueryCache>> = Mutex::new(None);

struct QueryCache {
    entries: HashMap<String, (Instant, String)>,
}

const CACHE_TTL: Duration = Duration::from_secs(300);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub fn query_codebase(
    repo_root: &Path,
    question: &str,
    budget: usize,
    graphify_bin: &str,
) -> Result<String> {
    let graph_path = repo_root.join("graphify-out").join("graph.json");
    if !graph_path.is_file() {
        bail!(
            "no graph at {} — run `agent-brain graphify enable` and build a graph first",
            graph_path.display()
        );
    }
    let key = cache_key(repo_root, question, budget);
    if let Some(cached) = cache_get(&key) {
        return Ok(cached);
    }
    let mut cmd = Command::new(graphify_bin);
    cmd.args(["query", question, "--budget", &budget.to_string()])
        .current_dir(repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = output_with_timeout(&mut cmd, QUERY_TIMEOUT)
        .with_context(|| format!("run {graphify_bin} query"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("graphify query failed: {stderr}");
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    cache_put(key, text.clone());
    Ok(text)
}

fn cache_key(repo_root: &Path, question: &str, budget: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.display().to_string().as_bytes());
    hasher.update(question.as_bytes());
    hasher.update(budget.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn cache_get(key: &str) -> Option<String> {
    let mut guard = QUERY_CACHE.lock().ok()?;
    let cache = guard.get_or_insert(QueryCache {
        entries: HashMap::new(),
    });
    let (ts, val) = cache.entries.get(key)?;
    if ts.elapsed() > CACHE_TTL {
        cache.entries.remove(key);
        return None;
    }
    Some(val.clone())
}

fn cache_put(key: String, value: String) {
    if let Ok(mut guard) = QUERY_CACHE.lock() {
        let cache = guard.get_or_insert(QueryCache {
            entries: HashMap::new(),
        });
        cache.entries.insert(key, (Instant::now(), value));
    }
}
