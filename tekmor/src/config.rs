use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
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
    /// Re-ingest session digests on route_task when last ingest is older than this (seconds).
    pub session_ingest_route_interval_secs: u64,
    pub workflow_dirs: Vec<PathBuf>,
    /// Write human-readable route summary to ~/.autonomic/memory/logs/last-route.md
    pub route_briefing_enabled: bool,
    /// One-line route summary on stderr (visible in Cursor MCP output)
    pub route_briefing_stderr: bool,
    /// Reject non-route_task MCP tools until route_task succeeds (all hosts).
    pub mcp_gate_enabled: bool,
    pub mcp_gate_ttl_secs: u64,
    /// Reuse prior route results within a session when scope/phase/files match (0 = off).
    pub session_stickiness_secs: u64,
    /// Lexical/ONNX-ready cross-encoder rerank of top-K (default off — must pass Recall@3 gate).
    pub precision_rerank_enabled: bool,
    /// Split multi-intent turns into sub-queries and RRF-fuse rankings (default off).
    pub query_decompose_enabled: bool,
    pub auto_trajectory_enabled: bool,
}

impl Config {
    pub fn load() -> Result<Self> {
        let explicit_home = std::env::var("TEKMOR_HOME").ok().map(PathBuf::from);
        let legacy_home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".tekmor");

        let (home, data_dir, logs_dir) = if let Some(explicit) = explicit_home.clone() {
            let data = explicit.join("data");
            let logs = explicit.join("logs");
            (explicit, data, logs)
        } else {
            let memory = crate::global_workspace::memory_dir();
            if let Err(err) = crate::global_workspace::migrate_legacy_storage(&legacy_home, &memory)
            {
                tracing::warn!(error = %err, "legacy memory migration skipped");
            }
            (memory.clone(), memory.clone(), memory.join("logs"))
        };

        let settings = crate::settings::AgentBrainSettings::load(&home);

        Ok(Self {
            db_path: data_dir.join("brain.db"),
            vectors_path: data_dir.join("vectors.bin"),
            turn_ttl_secs: env_u64("TEKMOR_TURN_TTL_SECS", settings.routing.turn_ttl_secs),
            auto_capture_enabled: env_bool("TEKMOR_AUTO_CAPTURE", false),
            auto_trajectory_enabled: env_bool(
                "TEKMOR_AUTO_TRAJECTORY",
                settings.routing.auto_trajectory,
            ),
            session_ingest_enabled: env_flag(
                std::env::var("TEKMOR_SESSION_INGEST").ok(),
                false,
            ),
            session_digest_enabled: env_flag(
                std::env::var("TEKMOR_SESSION_DIGEST").ok(),
                false,
            ),
            session_ingest_legacy: std::env::var("TEKMOR_SESSION_INGEST_LEGACY")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false),
            session_max_age_days: 90,
            prewarm_on_bootstrap: std::env::var("TEKMOR_PREWARM")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            bootstrap_background: std::env::var("TEKMOR_BOOTSTRAP_BG")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            embedding_cache_enabled: std::env::var("TEKMOR_EMBEDDING_CACHE")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            bm25_fast_path_enabled: std::env::var("TEKMOR_BM25_FAST_PATH")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            session_ingest_background: std::env::var("TEKMOR_SESSION_INGEST_BG")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
            turn_cache_ignore_open_files: std::env::var("TEKMOR_TURN_CACHE_OPEN_FILES")
                .map(|v| !(v == "0" || v == "false"))
                .unwrap_or(true),
            embedding_model: std::env::var("TEKMOR_EMBED_MODEL")
                .unwrap_or_else(|_| "mini".into()),
            bootstrap_startup_delay_secs: env_u64("TEKMOR_BOOTSTRAP_DELAY_SEC", 2),
            bootstrap_interval_secs: env_u64("TEKMOR_BOOTSTRAP_INTERVAL_SEC", 3600),
            auto_update_startup_delay_secs: env_u64("TEKMOR_AUTO_UPDATE_DELAY_SEC", 60),
            session_ingest_delay_secs: env_u64("TEKMOR_SESSION_INGEST_DELAY_SEC", 180),
            session_ingest_route_interval_secs: env_u64(
                "TEKMOR_SESSION_INGEST_ROUTE_INTERVAL",
                300,
            ),
            route_briefing_enabled: env_bool("TEKMOR_ROUTE_BRIEFING", true),
            route_briefing_stderr: env_bool("TEKMOR_ROUTE_BRIEFING_STDERR", true),
            mcp_gate_enabled: env_bool("TEKMOR_MCP_GATE", true),
            mcp_gate_ttl_secs: env_u64("TEKMOR_MCP_GATE_TTL", 14_400),
            session_stickiness_secs: env_u64(
                "TEKMOR_SESSION_STICKINESS_SECS",
                settings.routing.session_stickiness_secs,
            ),
            precision_rerank_enabled: env_bool("TEKMOR_PRECISION_RERANK", false),
            query_decompose_enabled: env_bool("TEKMOR_QUERY_DECOMPOSE", false),
            workflow_dirs: vec![home.join("workflows")],
            home,
            data_dir,
            logs_dir,
        })
    }


    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.home).context("create home dir")?;
        std::fs::create_dir_all(self.home.join("rules")).ok();
        std::fs::create_dir_all(self.home.join("skills")).ok();
        std::fs::create_dir_all(self.home.join("agents")).ok();
        let auto_root = crate::global_workspace::autonomic_root();
        if auto_root != self.home {
            std::fs::create_dir_all(auto_root.join("rules")).ok();
            std::fs::create_dir_all(auto_root.join("skills")).ok();
            std::fs::create_dir_all(auto_root.join("agents")).ok();
        }
        std::fs::create_dir_all(&self.data_dir).context("create data dir")?;
        std::fs::create_dir_all(&self.logs_dir).ok();
        std::fs::create_dir_all(self.home.join("export")).ok();
        std::fs::create_dir_all(self.home.join("workflows")).ok();
        Ok(())
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


fn env_flag(value: Option<String>, default: bool) -> bool {
    value.map(|v| v != "0" && v != "false").unwrap_or(default)
}

#[cfg(test)]
mod env_flag_tests {
    use super::env_flag;

    #[test]
    fn session_flags_default_off_when_unset() {
        assert!(!env_flag(None, false));
    }

    #[test]
    fn session_flags_turn_on_only_when_set_to_a_true_value() {
        assert!(env_flag(Some("1".into()), false));
        assert!(!env_flag(Some("0".into()), false));
        assert!(!env_flag(Some("false".into()), false));
    }
}

