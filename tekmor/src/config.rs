use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub db_path: PathBuf,
}

impl Config {
    pub fn load() -> Result<Self> {
        let home = match std::env::var("TEKMOR_HOME").ok().map(PathBuf::from) {
            Some(explicit) => explicit,
            None => crate::global_workspace::memory_dir(),
        };
        let data_dir = home.join("data");
        Ok(Self {
            db_path: data_dir.join("brain.db"),
            logs_dir: home.join("logs"),
            data_dir,
            home,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.home).context("create home dir")?;
        std::fs::create_dir_all(&self.data_dir).context("create data dir")?;
        std::fs::create_dir_all(&self.logs_dir).ok();
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
