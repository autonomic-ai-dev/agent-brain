use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "serve_meta.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServeMeta {
    pub version: String,
    pub exe: String,
    pub pid: u32,
    pub started_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeHealth {
    pub meta: Option<ServeMeta>,
    pub process_alive: bool,
    pub disk_version: Option<String>,
    pub disk_exe: Option<String>,
    pub stale: bool,
}

pub fn write_current(home: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let meta = ServeMeta {
        version: env!("CARGO_PKG_VERSION").into(),
        exe: exe.display().to_string(),
        pid: std::process::id(),
        started_unix: chrono::Utc::now().timestamp(),
    };
    let path = home.join(FILE_NAME);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create serve_meta parent")?;
    }
    let pretty = serde_json::to_string_pretty(&meta)?;
    fs::write(path, format!("{pretty}\n")).context("write serve_meta.json")
}

pub fn load(home: &Path) -> Option<ServeMeta> {
    let path = home.join(FILE_NAME);
    if !path.is_file() {
        return None;
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub fn binary_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}

pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

pub fn assess(home: &Path, mcp_binary: Option<&Path>) -> ServeHealth {
    let meta = load(home);
    let process_alive = meta.as_ref().map(|m| process_alive(m.pid)).unwrap_or(false);

    let disk_exe = mcp_binary.map(|p| p.display().to_string());
    let disk_version = mcp_binary.and_then(binary_version);

    let stale = match (&meta, &disk_version) {
        (Some(m), Some(disk)) if process_alive => m.version != *disk,
        (Some(m), None) if process_alive => {
            mcp_binary.is_some_and(|p| fs::canonicalize(p).ok() != fs::canonicalize(&m.exe).ok())
        }
        _ => false,
    };

    ServeHealth {
        meta,
        process_alive,
        disk_version,
        disk_exe,
        stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_meta() {
        let dir = TempDir::new().unwrap();
        write_current(dir.path()).unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.version, env!("CARGO_PKG_VERSION"));
        assert!(loaded.pid > 0);
    }

    #[test]
    fn stale_when_versions_differ() {
        let health = ServeHealth {
            meta: Some(ServeMeta {
                version: "0.7.0".into(),
                exe: "/tmp/tekmor".into(),
                pid: 1,
                started_unix: 0,
            }),
            process_alive: true,
            disk_version: Some("0.7.2".into()),
            disk_exe: Some("/tmp/tekmor".into()),
            stale: true,
        };
        assert!(health.stale);
    }
}
