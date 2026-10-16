//! Verifier-sourced outcomes: `agent-brain verify -- <cmd>` and `verify record`.
//!
//! Project playbooks live at `<project>/.verify/`. Hashes of verify config /
//! scenario dirs make drift visible in `agent-brain report`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::db::store::BrainStore;

const MIN_SAMPLE_FOR_RATES: usize = 30;
pub const VERIFY_DIR_NAME: &str = ".verify";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifySource {
    /// `agent-brain verify -- <cmd>` ran the process and recorded the real exit.
    Cli,
    /// Host Shell PostToolUse / hook captured a real exit.
    Hook,
    /// MCP `record_verify_outcome` (agent-asserted; not trusted for learning).
    Mcp,
    /// Explicit playbook-driven record.
    Playbook,
    /// Spine/muscle/orchestrator invoked `verify --` (trusted; distinct from agent CLI).
    Orchestrator,
}

impl VerifySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Hook => "hook",
            Self::Mcp => "mcp",
            Self::Playbook => "playbook",
            Self::Orchestrator => "orchestrator",
        }
    }

    /// Full learning weight: orchestrator, cli (allowlisted), hook (allowlisted).
    pub fn is_trusted_for_learning(self) -> bool {
        matches!(self, Self::Cli | Self::Hook | Self::Orchestrator)
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cli" => Ok(Self::Cli),
            "hook" => Ok(Self::Hook),
            "mcp" => Ok(Self::Mcp),
            "playbook" => Ok(Self::Playbook),
            "orchestrator" | "spine" | "muscle" => Ok(Self::Orchestrator),
            other => bail!(
                "unknown verify source `{other}` (expected cli|hook|mcp|playbook|orchestrator)"
            ),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct VerifyTomlFile {
    #[serde(default)]
    verify: VerifyTomlSection,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct VerifyTomlSection {
    #[serde(default)]
    commands: Vec<String>,
}

/// Load project command allowlist from `<root>/.verify/verify.toml` or `<root>/verify.toml`.
pub fn load_command_allowlist(project_root: &Path) -> Vec<String> {
    for rel in [".verify/verify.toml", "verify.toml"] {
        let path = project_root.join(rel);
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(parsed) = toml::from_str::<VerifyTomlFile>(&raw) {
            if !parsed.verify.commands.is_empty() {
                return parsed.verify.commands;
            }
        }
    }
    Vec::new()
}

/// True if `cmd` matches an allowlist entry (substring or as prefix of the joined command).
/// Empty allowlist → fallback to common runners (cargo test, pytest, .verify/, mvnw verify).
pub fn command_is_allowlisted(cmd: &str, project_root: Option<&Path>) -> bool {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return false;
    }
    let list = project_root
        .map(load_command_allowlist)
        .unwrap_or_default();
    if list.is_empty() {
        return default_allowlist_match(cmd);
    }
    list.iter().any(|entry| {
        let e = entry.trim();
        !e.is_empty() && (cmd == e || cmd.starts_with(e) || cmd.contains(e))
    })
}

fn default_allowlist_match(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.contains("cargo test")
        || lower.contains("cargo clippy")
        || lower.contains("cargo check")
        || lower.contains("pytest")
        || lower.contains("npm test")
        || lower.contains("bun test")
        || lower.contains("go test")
        || lower.contains("make test")
        || lower.contains("./mvnw")
        || lower.contains("mvnw ")
        || lower.contains(".verify/")
        || lower.contains("agent-brain proofs")
        || lower.contains("agent-brain doctor")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyReport {
    pub id: String,
    pub exit_code: i32,
    pub success: bool,
    pub cause_tag: Option<String>,
    pub verify_config_hash: Option<String>,
    pub scenario_dir_hash: Option<String>,
    pub cmd: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_root: Option<String>,
}

/// Walk ancestors of `start` for a directory containing `.verify/`.
pub fn find_verify_root(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        let candidate = cur.join(VERIFY_DIR_NAME);
        if candidate.is_dir() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn resolve_scenarios_dir(
    explicit: Option<&Path>,
    cwd: Option<&Path>,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    if let Some(dir) = explicit {
        return Ok((Some(dir.to_path_buf()), None));
    }
    let start = cwd
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if let Some(root) = find_verify_root(&start) {
        let verify_dir = root.join(VERIFY_DIR_NAME);
        return Ok((Some(verify_dir), Some(root)));
    }
    Ok((None, None))
}

/// Record an already-known exit code (no command spawn).
pub fn record_from_result(
    store: &BrainStore,
    exit_code: i32,
    cmd: &str,
    route_log_id: Option<&str>,
    cause_tag: Option<&str>,
    config_path: Option<&Path>,
    scenarios_dir: Option<&Path>,
    cwd: Option<&Path>,
    source: VerifySource,
) -> Result<VerifyReport> {
    if cmd.trim().is_empty() {
        bail!("verify record requires a non-empty cmd summary after --");
    }
    let success = exit_code == 0;
    let config_hash = config_path.map(hash_path).transpose()?;
    let (scenarios, verify_root) = resolve_scenarios_dir(scenarios_dir, cwd)?;
    let scenarios_hash = scenarios
        .as_ref()
        .map(|p| hash_tree(p))
        .transpose()?;
    let cause = cause_tag.map(|s| s.to_string()).or_else(|| {
        if success {
            None
        } else {
            Some(cmd.to_string())
        }
    });
    let device_id = store.ensure_device_id()?;
    let id = store.insert_verify_outcome(
        route_log_id,
        exit_code,
        cause.as_deref(),
        config_hash.as_deref(),
        scenarios_hash.as_deref(),
        cmd,
        &device_id,
        source.as_str(),
    )?;
    Ok(VerifyReport {
        id,
        exit_code,
        success,
        cause_tag: cause,
        verify_config_hash: config_hash,
        scenario_dir_hash: scenarios_hash,
        cmd: cmd.to_string(),
        source: source.as_str().to_string(),
        verify_root: verify_root.map(|p| p.display().to_string()),
    })
}

pub fn run_verify(
    store: &BrainStore,
    cmd: &[String],
    route_log_id: Option<&str>,
    cause_tag: Option<&str>,
    config_path: Option<&Path>,
    scenarios_dir: Option<&Path>,
    source: VerifySource,
) -> Result<VerifyReport> {
    if cmd.is_empty() {
        bail!("usage: agent-brain verify [--source cli|orchestrator] [--config PATH] [--scenarios DIR] [--route-log-id ID] [--cause TAG] -- <cmd> [args...]");
    }
    let source = match source {
        VerifySource::Mcp | VerifySource::Playbook | VerifySource::Hook => VerifySource::Cli,
        other => other,
    };
    let status = Command::new(&cmd[0])
        .args(&cmd[1..])
        .status()
        .with_context(|| format!("failed to spawn {}", cmd[0]))?;
    let exit_code = status.code().unwrap_or(1);
    let cwd = std::env::current_dir().ok();
    record_from_result(
        store,
        exit_code,
        &cmd.join(" "),
        route_log_id,
        cause_tag,
        config_path,
        scenarios_dir,
        cwd.as_deref(),
        source,
    )
}

pub fn print_report(store: &BrainStore) -> Result<()> {
    let rows = store.list_verify_outcomes(500)?;
    let n = rows.len();
    let success = rows.iter().filter(|r| r.exit_code == 0).count();
    let fail = n.saturating_sub(success);
    println!("verify outcomes: n={n} success={success} failure={fail}");

    let mut by_source: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for r in &rows {
        *by_source
            .entry(r.source.clone().unwrap_or_else(|| "cli".to_string()))
            .or_default() += 1;
    }
    if !by_source.is_empty() {
        let parts: Vec<String> = by_source
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        println!("by_source: {{{}}}", parts.join(", "));
    }

    if n < MIN_SAMPLE_FOR_RATES {
        println!("rates suppressed (need n>={MIN_SAMPLE_FOR_RATES}; showing counts only)");
    } else {
        let rate = success as f64 / n as f64;
        println!("first-pass success rate: {rate:.3}");
    }

    let mut config_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for r in &rows {
        if let Some(h) = &r.verify_config_hash {
            *config_counts.entry(h.clone()).or_default() += 1;
        }
    }
    if let Some((baseline, _)) = config_counts.iter().max_by_key(|(_, c)| *c) {
        let drifted: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.verify_config_hash
                    .as_ref()
                    .is_some_and(|h| h != baseline)
            })
            .collect();
        if !drifted.is_empty() {
            println!(
                "WARNING: {} runs used a different verify_config_hash than the modal baseline",
                drifted.len()
            );
        }
    }

    println!(
        "note: source=orchestrator|cli (allowlisted verify --) strongest; hook observational; mcp/playbook not trusted for learning"
    );
    Ok(())
}

fn hash_path(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn hash_tree(dir: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut paths: Vec<_> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    paths.sort();
    for path in paths {
        let rel = path.strip_prefix(dir).unwrap_or(&path);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(fs::read(&path)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::BrainStore;
    use tempfile::tempdir;

    #[test]
    fn find_verify_root_walks_ancestors() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        assert!(find_verify_root(&nested).is_none());
        fs::create_dir_all(dir.path().join(".verify")).unwrap();
        assert_eq!(find_verify_root(&nested).as_deref(), Some(dir.path()));
    }

    #[test]
    fn record_from_result_stores_source_and_verify_hash() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = BrainStore::open(&db).unwrap();
        fs::create_dir_all(dir.path().join(".verify")).unwrap();
        fs::write(dir.path().join(".verify/README.md"), b"playbook").unwrap();

        let report = record_from_result(
            &store,
            0,
            "cargo test",
            None,
            None,
            None,
            None,
            Some(dir.path()),
            VerifySource::Hook,
        )
        .unwrap();
        assert_eq!(report.source, "hook");
        assert!(report.scenario_dir_hash.is_some());
        assert_eq!(report.verify_root.as_deref(), Some(dir.path().to_str().unwrap()));

        let rows = store.list_verify_outcomes(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source.as_deref(), Some("hook"));
        assert!(rows[0].scenario_dir_hash.is_some());
    }

    #[test]
    fn orchestrator_source_parses_and_is_trusted() {
        assert_eq!(
            VerifySource::parse("orchestrator").unwrap(),
            VerifySource::Orchestrator
        );
        assert_eq!(VerifySource::parse("spine").unwrap(), VerifySource::Orchestrator);
        assert!(VerifySource::Orchestrator.is_trusted_for_learning());
        assert!(!VerifySource::Mcp.is_trusted_for_learning());
    }

    #[test]
    fn allowlist_from_verify_toml_and_rejects_true() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".verify")).unwrap();
        fs::write(
            dir.path().join(".verify/verify.toml"),
            r#"
[verify]
commands = ["./mvnw -q verify", ".verify/run.sh", "cargo test"]
"#,
        )
        .unwrap();
        assert!(command_is_allowlisted(
            "./mvnw -q verify -DskipITs",
            Some(dir.path())
        ));
        assert!(command_is_allowlisted("cargo test -p agent-brain", Some(dir.path())));
        assert!(!command_is_allowlisted("true", Some(dir.path())));
        assert!(!command_is_allowlisted("echo hi", Some(dir.path())));
    }
}
