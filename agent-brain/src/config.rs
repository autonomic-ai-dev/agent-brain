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
    pub session_digest_enabled: bool,
    pub session_ingest_legacy: bool,
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
    /// Write human-readable route summary to ~/.agent_brain/logs/last-route.md
    pub route_briefing_enabled: bool,
    /// One-line route summary on stderr (visible in Cursor MCP output)
    pub route_briefing_stderr: bool,
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
            session_digest_enabled: std::env::var("AGENT_BRAIN_SESSION_DIGEST")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            session_ingest_legacy: std::env::var("AGENT_BRAIN_SESSION_INGEST_LEGACY")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false),
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
            auto_update_startup_delay_secs: env_u64("AGENT_BRAIN_AUTO_UPDATE_DELAY_SEC", 60),
            session_ingest_delay_secs: env_u64("AGENT_BRAIN_SESSION_INGEST_DELAY_SEC", 180),
            route_briefing_enabled: env_bool("AGENT_BRAIN_ROUTE_BRIEFING", true),
            route_briefing_stderr: env_bool("AGENT_BRAIN_ROUTE_BRIEFING_STDERR", true),
            home,
            data_dir,
        })
    }

    /// Ephemeral home for eval/bench proofs — no production `~/.agent_brain` reads or writes.
    pub fn isolated(home: PathBuf) -> Self {
        let data_dir = home.join("data");
        Self {
            home: home.clone(),
            data_dir: data_dir.clone(),
            db_path: data_dir.join("brain.db"),
            vectors_path: data_dir.join("vectors.bin"),
            turn_ttl_secs: 60,
            auto_capture_enabled: false,
            session_ingest_enabled: false,
            session_digest_enabled: false,
            session_ingest_legacy: false,
            session_max_age_days: 90,
            prewarm_on_bootstrap: false,
            bootstrap_background: false,
            embedding_cache_enabled: true,
            bm25_fast_path_enabled: false,
            session_ingest_background: false,
            turn_cache_ignore_open_files: true,
            embedding_model: "mini".into(),
            bootstrap_startup_delay_secs: 0,
            bootstrap_interval_secs: 0,
            auto_update_startup_delay_secs: 0,
            session_ingest_delay_secs: 0,
            route_briefing_enabled: false,
            route_briefing_stderr: false,
        }
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

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}
