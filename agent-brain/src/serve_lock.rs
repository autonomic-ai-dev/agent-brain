//! Exclusive advisory lock so only one `agent-brain serve` owns a brain home.
//!
//! Multiple stdio MCP servers on the same `brain.db` do not share pipes, but they
//! do share SQLite + the Python hook `route_state.json`. Concurrent hosts were a
//! major source of mid-turn "Tool execution error" / route-gate flakes.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub fn lock_identity(home: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(home.join("serve.lock"))
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

pub struct ServeLock {
    _file: File,
    path: PathBuf,
}

impl Drop for ServeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Acquire an exclusive non-blocking lock on `<home>/serve.lock`.
///
/// Returns `Ok(None)` when `AGENT_BRAIN_ALLOW_MULTI_SERVE` is set and another
/// serve already holds the lock (caller proceeds with a warning).
pub fn acquire(home: &Path) -> Result<Option<ServeLock>> {
    std::fs::create_dir_all(home).ok();
    let path = home.join("serve.lock");
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("open serve lock {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // LOCK_EX | LOCK_NB
        const LOCK_EX: i32 = 2;
        const LOCK_NB: i32 = 4;
        let rc = unsafe { libc::flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
        if rc != 0 {
            let allow = std::env::var("AGENT_BRAIN_ALLOW_MULTI_SERVE")
                .map(|v| {
                    let v = v.trim().to_ascii_lowercase();
                    matches!(v.as_str(), "1" | "true" | "yes" | "on")
                })
                .unwrap_or(false);
            let holder = std::fs::read_to_string(&path).unwrap_or_default();
            let msg = format!(
                "another agent-brain serve already holds {} (holder: {})",
                path.display(),
                holder.trim()
            );
            if allow {
                eprintln!(
                    "agent-brain: WARNING: {msg}; continuing because AGENT_BRAIN_ALLOW_MULTI_SERVE=1. \
                     Concurrent serves share brain.db and can re-arm the route gate mid-turn."
                );
                return Ok(None);
            }
            bail!(
                "{msg}\n\
                 Stop the other process (or `kill $(cat {})`), or set AGENT_BRAIN_ALLOW_MULTI_SERVE=1 \
                 only if you accept mid-turn MCP flakes.",
                path.display()
            );
        }
    }

    #[cfg(not(unix))]
    {
        let _ = &file;
    }

    file.set_len(0)?;
    writeln!(
        file,
        "pid={} version={}",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    )?;
    file.flush()?;

    Ok(Some(ServeLock { _file: file, path }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lock_identity_changes_when_the_lock_file_is_replaced() {
        let dir = tempdir().unwrap();
        let held = acquire(dir.path()).expect("acquire").expect("lock");
        let first = lock_identity(dir.path());
        assert!(first.is_some());
        assert_eq!(first, lock_identity(dir.path()));
        std::fs::remove_file(dir.path().join("serve.lock")).unwrap();
        std::fs::write(dir.path().join("serve.lock"), b"replacement").unwrap();
        assert_ne!(first, lock_identity(dir.path()));
        drop(held);
    }

    #[test]
    fn acquire_writes_pid_and_drop_removes_lock_file() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("serve.lock");
        {
            let held = acquire(dir.path()).expect("acquire");
            assert!(held.is_some());
            let body = std::fs::read_to_string(&lock_path).expect("read lock");
            assert!(body.contains("pid="));
            assert!(lock_path.exists());
        }
        assert!(
            !lock_path.exists(),
            "ServeLock drop should remove serve.lock"
        );
    }
}
