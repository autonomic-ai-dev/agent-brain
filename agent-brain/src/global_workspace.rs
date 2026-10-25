use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn autonomic_root() -> PathBuf {
    std::env::var("AUTONOMIC_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".autonomic"))
}

fn home_dir() -> PathBuf {
    dirs::home_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn memory_dir() -> PathBuf {
    autonomic_root().join("memory")
}

pub fn memory_logs_dir() -> PathBuf {
    memory_dir().join("logs")
}

pub fn spine_logs_dir() -> PathBuf {
    autonomic_root().join("logs").join("spine")
}

pub fn executions_dir() -> PathBuf {
    spine_logs_dir().join("executions")
}

pub fn agents_md_path() -> PathBuf {
    autonomic_root().join("AGENTS.md")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    fs::create_dir_all(memory_dir())?;
    fs::create_dir_all(memory_logs_dir())?;
    fs::create_dir_all(spine_logs_dir())?;
    fs::create_dir_all(executions_dir())?;
    fs::create_dir_all(autonomic_root())?;
    Ok(())
}

/// Copy legacy `~/.agent_brain/data` and logs into `~/.autonomic/memory` when empty.
pub fn migrate_legacy_storage(legacy_home: &Path, memory_dir: &Path) -> Result<()> {
    fs::create_dir_all(memory_dir).context("create global memory dir")?;
    let legacy_data = legacy_home.join("data");
    let legacy_logs = legacy_home.join("logs");
    let dst_logs = memory_dir.join("logs");

    for name in ["brain.db", "vectors.bin"] {
        let src = legacy_data.join(name);
        let dst = memory_dir.join(name);
        if src.is_file() && !dst.exists() {
            fs::copy(&src, &dst).with_context(|| format!("migrate {name}"))?;
            tracing::info!(from = %src.display(), to = %dst.display(), "migrated to global workspace");
        }
    }

    if legacy_logs.is_dir() {
        fs::create_dir_all(&dst_logs)?;
        for entry in fs::read_dir(&legacy_logs)? {
            let entry = entry?;
            let dst = dst_logs.join(entry.file_name());
            if entry.path().is_file() && !dst.exists() {
                fs::copy(entry.path(), &dst)?;
            }
        }
    }

    for dir in &["packages", "export", "workflows"] {
        let src = legacy_home.join(dir);
        let dst = memory_dir.join(dir);
        if src.is_dir() && !dst.exists() {
            copy_dir_all(&src, &dst).with_context(|| format!("migrate {dir} from legacy"))?;
            tracing::info!(from = %src.display(), to = %dst.display(), "migrated {dir}");
        }
    }

    let legacy_settings = legacy_home.join("config.yaml");
    let dst_settings = memory_dir.join("config.yaml");
    if legacy_settings.is_file() && !dst_settings.exists() {
        fs::copy(&legacy_settings, &dst_settings)?;
        tracing::info!(from = %legacy_settings.display(), to = %dst_settings.display(), "migrated config");
    }

    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else if file_type.is_file() && !dst_path.exists() {
            fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrate_copies_brain_db_when_missing_in_global_workspace() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let memory = tmp.path().join("memory");
        fs::create_dir_all(legacy.join("data")).unwrap();
        fs::write(legacy.join("data/brain.db"), b"sqlite").unwrap();

        migrate_legacy_storage(&legacy, &memory).unwrap();

        assert!(memory.join("brain.db").is_file());
    }
}
