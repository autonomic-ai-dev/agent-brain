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
    pub embedding_cache_enabled: bool,
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
            embedding_cache_enabled: std::env::var("AGENT_BRAIN_EMBEDDING_CACHE")
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true),
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
