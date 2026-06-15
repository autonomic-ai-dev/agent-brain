use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub vectors_path: PathBuf,
    pub turn_ttl_secs: u64,
    pub auto_capture_enabled: bool,
    pub session_ingest_enabled: bool,
    pub session_max_age_days: u64,
    pub prewarm_on_bootstrap: bool,
    pub bootstrap_background: bool,
    pub embedding_cache_enabled: bool,
    pub bm25_fast_path_enabled: bool,
    pub session_ingest_background: bool,
    pub turn_cache_ignore_open_files: bool,
    pub embedding_model: String,
    /// Seconds to wait after `serve` before background bootstrap (lets MCP handshake finish).
    pub bootstrap_startup_delay_secs: u64,
    /// Skip background bootstrap when last run was within this many seconds (0 = always run).
    pub bootstrap_interval_secs: u64,
    /// Seconds to wait after `serve` before auto-update checks.
    pub auto_update_startup_delay_secs: u64,
    /// Extra delay before background session ingest (after bootstrap).
    pub session_ingest_delay_secs: u64,
}

impl Config {
    pub fn load() -> Result<Self> {
        let home = std::env::var("AGENT_BRAIN_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".agent_brain")
            });

        let data_dir = home.join("data");
        Ok(Self {
            db_path: data_dir.join("brain.db"),
            vectors_path: data_dir.join("vectors.bin"),
            turn_ttl_secs: 60,
            auto_capture_enabled: true,
            session_ingest_enabled: std::env::var("AGENT_BRAIN_SESSION_INGEST")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            session_max_age_days: 90,
            prewarm_on_bootstrap: std::env::var("AGENT_BRAIN_PREWARM")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            bootstrap_background: std::env::var("AGENT_BRAIN_BOOTSTRAP_BG")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            embedding_cache_enabled: std::env::var("AGENT_BRAIN_EMBEDDING_CACHE")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            bm25_fast_path_enabled: std::env::var("AGENT_BRAIN_BM25_FAST_PATH")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            session_ingest_background: std::env::var("AGENT_BRAIN_SESSION_INGEST_BG")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            turn_cache_ignore_open_files: std::env::var("AGENT_BRAIN_TURN_CACHE_OPEN_FILES")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false),
            embedding_model: std::env::var("AGENT_BRAIN_EMBED_MODEL").unwrap_or_else(|_| "mini".into()),
            bootstrap_startup_delay_secs: env_u64("AGENT_BRAIN_BOOTSTRAP_DELAY_SEC", 2),
            bootstrap_interval_secs: env_u64("AGENT_BRAIN_BOOTSTRAP_INTERVAL_SEC", 3600),
            auto_update_startup_delay_secs: env_u64("AGENT_BRAIN_AUTO_UPDATE_DELAY_SEC", 300),
            session_ingest_delay_secs: env_u64("AGENT_BRAIN_SESSION_INGEST_DELAY_SEC", 180),
            home,
            data_dir,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.home).context("create home dir")?;
        std::fs::create_dir_all(self.home.join("rules")).ok();
        std::fs::create_dir_all(self.home.join("skills")).ok();
        std::fs::create_dir_all(self.home.join("agents")).ok();
        std::fs::create_dir_all(&self.data_dir).context("create data dir")?;
        std::fs::create_dir_all(self.home.join("logs")).ok();
        std::fs::create_dir_all(self.home.join("export")).ok();
        std::fs::create_dir_all(self.home.join("packages")).ok();
        Ok(())
    }

    pub fn default_index_roots(&self, cwd: Option<&Path>) -> Vec<PathBuf> {
        let mut roots = Vec::new();

        roots.push(self.home.join("rules"));
        roots.push(self.home.join("skills"));
        roots.push(self.home.join("agents"));
        roots.extend(crate::packages::package_index_roots(&self.home));

        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".cursor/skills-cursor"));
            roots.push(home.join(".cursor/skills"));
            roots.push(home.join(".claude/skills"));
            roots.push(home.join(".claude/agents"));
            roots.push(home.join(".codex/skills"));
            roots.push(home.join(".codex/agents"));

            if let Ok(entries) = glob::glob(&format!(
                "{}/.cursor/plugins/**/agents/*.md",
                home.display()
            )) {
                for entry in entries.flatten() {
                    if let Some(parent) = entry.parent() {
                        roots.push(parent.to_path_buf());
                    }
                }
            }
        }

        if let Some(cwd) = cwd {
            if let Some(repo) = find_repo_root(cwd) {
                roots.push(repo.join(".cursor/rules"));
                roots.push(repo.join(".cursor/agents"));
                roots.push(repo.join(".claude/agents"));
                for name in ["CLAUDE.md", "AGENTS.md", ".cursorrules"] {
                    let p = repo.join(name);
                    if p.is_file() {
                        roots.push(p);
                    }
                }
            }
        }

        roots
    }
}

pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return Some(start.to_path_buf());
        }
    }
}

pub fn expand_home(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(stripped)
    } else {
        PathBuf::from(path)
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
