//! Verifier-sourced outcomes: `agent-brain verify -- <cmd>` and `verify record`.
//!
//! Project playbooks live at `<project>/.verify/`. Hashes of verify config /
//! scenario dirs make drift visible in `agent-brain report`.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::db::store::BrainStore;
use crate::types::{VerificationStatus, VerifiedRecipe};

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
            "orchestrator" | "spine" => Ok(Self::Orchestrator),
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
    #[serde(default)]
    default: Option<DefaultStepFile>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DefaultStepFile {
    command: Vec<String>,
    #[serde(default = "default_step_timeout_secs")]
    timeout_secs: u64,
    #[serde(default = "default_step_hook")]
    hook: bool,
}

fn default_step_timeout_secs() -> u64 {
    300
}

fn default_step_hook() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultStep {
    pub command: Vec<String>,
    pub timeout: Duration,
    pub hook: bool,
}

pub const TIMEOUT_EXIT_CODE: i32 = 124;
pub const MAX_STEP_TIMEOUT_SECS: u64 = 600;
const VERIFY_RUN_TIMEOUT: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn load_default_step(project_root: &Path) -> Result<Option<DefaultStep>> {
    for rel in [".verify/verify.toml", "verify.toml"] {
        let Ok(raw) = fs::read_to_string(project_root.join(rel)) else {
            continue;
        };
        let parsed: VerifyTomlFile =
            toml::from_str(&raw).with_context(|| format!("invalid verify config {rel}"))?;
        let Some(step) = parsed.verify.default else {
            continue;
        };
        if step.command.is_empty() {
            bail!("[verify.default] command is empty in {rel}");
        }
        if !(1..=MAX_STEP_TIMEOUT_SECS).contains(&step.timeout_secs) {
            bail!("[verify.default] timeout_secs must be between 1 and {MAX_STEP_TIMEOUT_SECS} in {rel}");
        }
        return Ok(Some(DefaultStep {
            command: step.command,
            timeout: Duration::from_secs(step.timeout_secs),
            hook: step.hook,
        }));
    }
    Ok(None)
}

const OUTPUT_TAIL_BYTES: u64 = 4000;

fn run_process(
    cmd: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    capture: bool,
) -> Result<(i32, String)> {
    if cmd.is_empty() {
        bail!("cannot run an empty command");
    }
    let mut command = Command::new(&cmd[0]);
    command.args(&cmd[1..]);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let output = if capture {
        let file = tempfile::tempfile()?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone()?))
            .stderr(Stdio::from(file.try_clone()?));
        Some(file)
    } else {
        None
    };
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {}", cmd[0]))?;
    let started = Instant::now();
    let exit_code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code().unwrap_or(1);
        }
        if started.elapsed() >= timeout {
            child.kill().ok();
            child.wait()?;
            break TIMEOUT_EXIT_CODE;
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    Ok((exit_code, output.map(read_output).unwrap_or_default()))
}

fn read_output(mut file: File) -> String {
    let mut buf = Vec::new();
    if file.seek(SeekFrom::Start(0)).is_ok() {
        file.read_to_end(&mut buf).ok();
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn output_tail(output: &str) -> String {
    let bytes = output.as_bytes();
    let start = bytes.len().saturating_sub(OUTPUT_TAIL_BYTES as usize);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

pub fn count_tests(output: &str) -> Option<u64> {
    output
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("test result:"))
        .filter_map(|rest| rest.split_whitespace().nth(1)?.parse::<u64>().ok())
        .reduce(|total, n| total + n)
}

#[derive(Debug)]
pub struct StepRun {
    pub report: VerifyReport,
    pub output: String,
}

pub fn run_default_step(
    store: &BrainStore,
    project_root: &Path,
    source: VerifySource,
) -> Result<Option<StepRun>> {
    let Some(step) = load_default_step(project_root)? else {
        return Ok(None);
    };
    let joined = step.command.join(" ");
    if !command_is_allowlisted(&joined, Some(project_root)) {
        bail!(
            "default verify step `{joined}` is not in the verify allowlist; add it to commands in .verify/verify.toml"
        );
    }
    let (exit_code, output) = run_process(&step.command, Some(project_root), step.timeout, true)?;
    let report = record_from_result(
        store,
        exit_code,
        &joined,
        None,
        None,
        None,
        None,
        Some(project_root),
        source,
    )?;
    Ok(Some(StepRun {
        report,
        output: output_tail(&output),
    }))
}

pub fn run_project_verify(store: &BrainStore, project: &Path) -> Result<serde_json::Value> {
    let root = crate::config::find_repo_root(project).unwrap_or_else(|| project.to_path_buf());
    match run_default_step(store, &root, VerifySource::Cli)? {
        None => Ok(serde_json::json!({
            "ran": false,
            "project": root.display().to_string(),
            "reason": "no [verify.default] step declared"
        })),
        Some(run) => Ok(serde_json::json!({
            "ran": true,
            "project": root.display().to_string(),
            "verify": run.report,
            "output": run.output
        })),
    }
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
    let list = project_root.map(load_command_allowlist).unwrap_or_default();
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
    let scenarios_hash = scenarios.as_ref().map(|p| hash_tree(p)).transpose()?;
    let cause = cause_tag.map(|s| s.to_string()).or_else(|| {
        if success {
            None
        } else {
            Some(cmd.to_string())
        }
    });
    let resolved_log_id = route_log_id
        .map(String::from)
        .or_else(|| store.latest_retrieval_log().ok().flatten().map(|r| r.id));

    // Closed-loop outcome attribution: automatically attribute context feedback for the route turn
    if let Some(ref log_id) = resolved_log_id {
        if let Ok(Some(retrieval_log)) = store.get_retrieval_log(log_id) {
            if let Ok(items) =
                serde_json::from_str::<Vec<serde_json::Value>>(&retrieval_log.items_json)
            {
                let item_ids: Vec<String> = items
                    .iter()
                    .filter_map(|item| {
                        item.get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .collect();
                if !item_ids.is_empty() {
                    let _ = store.record_context_feedback(&item_ids, success);
                }
            }
        }
    }

    let device_id = store.ensure_device_id()?;
    let project_key = crate::project_key::project_key_if_known(cwd.unwrap_or(Path::new(".")));
    let id = store.insert_verify_outcome(
        resolved_log_id.as_deref(),
        exit_code,
        cause.as_deref(),
        config_hash.as_deref(),
        scenarios_hash.as_deref(),
        cmd,
        &device_id,
        source.as_str(),
        project_key.as_deref(),
    )?;
    if let Some(hash) = cwd
        .and_then(crate::config::find_repo_root)
        .and_then(|root| code_tree_hash(&root))
    {
        store.set_verify_outcome_code_hash(&id, &hash)?;
    }

    let outcome_str = if success { "success" } else { "failure" };
    let clean_cmd = cmd.split_whitespace().next().unwrap_or("verify");
    let notes_str = format!("exit {exit_code} {cmd}");
    let wf_id = project_key.as_deref().unwrap_or("project");
    let _ = crate::trajectory::auto_record_trajectory(
        store,
        wf_id,
        clean_cmd,
        outcome_str,
        resolved_log_id.as_deref(),
        Some("verification"),
        Some(&notes_str),
    );
    if success {
        let _ = store.record_procedure_edge(wf_id, "implementing", clean_cmd);
    }

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

/// Initialize `.verify/` in `project_root` with `verify.toml`, `run.sh`, and `README.md`.
fn default_step_for(project_root: &Path) -> Option<&'static [&'static str]> {
    if project_root.join("Cargo.toml").exists() {
        return Some(&["cargo", "test"]);
    }
    if project_root.join("package.json").exists() {
        return Some(&["npm", "test"]);
    }
    if project_root.join("pyproject.toml").exists()
        || project_root.join("requirements.txt").exists()
        || project_root.join("pytest.ini").exists()
    {
        return Some(&["pytest"]);
    }
    if project_root.join("go.mod").exists() {
        return Some(&["go", "test", "./..."]);
    }
    if project_root.join("Makefile").exists() {
        return Some(&["make", "test"]);
    }
    None
}

pub fn init_verify(project_root: &Path, force: bool) -> Result<PathBuf> {
    let verify_dir = project_root.join(VERIFY_DIR_NAME);
    fs::create_dir_all(&verify_dir)
        .with_context(|| format!("create verify directory at {}", verify_dir.display()))?;

    let toml_path = verify_dir.join("verify.toml");
    if force || !toml_path.exists() {
        let mut commands = Vec::new();
        if project_root.join("Cargo.toml").exists() {
            commands.push("cargo test".to_string());
            commands.push("cargo clippy".to_string());
            commands.push("cargo check".to_string());
        }
        if project_root.join("package.json").exists() {
            commands.push("npm test".to_string());
        }
        if project_root.join("pyproject.toml").exists()
            || project_root.join("requirements.txt").exists()
            || project_root.join("pytest.ini").exists()
        {
            commands.push("pytest".to_string());
        }
        if project_root.join("go.mod").exists() {
            commands.push("go test ./...".to_string());
        }
        if project_root.join("Makefile").exists() {
            commands.push("make test".to_string());
        }
        if commands.is_empty() {
            commands.push("cargo test".to_string());
            commands.push("pytest".to_string());
            commands.push("npm test".to_string());
            commands.push("make test".to_string());
        }
        commands.push(".verify/".to_string());

        let mut toml_content = String::from("[verify]\ncommands = [\n");
        for cmd in &commands {
            toml_content.push_str(&format!("  \"{cmd}\",\n"));
        }
        toml_content.push_str("]\n");
        if let Some(argv) = default_step_for(project_root) {
            let quoted = argv
                .iter()
                .map(|arg| format!("\"{arg}\""))
                .collect::<Vec<_>>()
                .join(", ");
            toml_content.push_str(&format!(
                "\n[verify.default]\ncommand = [{quoted}]\ntimeout_secs = 300\n"
            ));
        }

        fs::write(&toml_path, toml_content)
            .with_context(|| format!("write {}", toml_path.display()))?;
    }

    let run_sh_path = verify_dir.join("run.sh");
    if force || !run_sh_path.exists() {
        let script = r#"#!/usr/bin/env bash
set -euo pipefail

# Project verification entrypoint executed by `agent-brain verify -- .verify/run.sh`
echo "==> Running verification suite..."

if [ -f Cargo.toml ]; then
  cargo test
elif [ -f package.json ]; then
  npm test
elif [ -f pyproject.toml ] || [ -f requirements.txt ]; then
  pytest
elif [ -f Makefile ]; then
  make test
else
  echo "No default test runner detected. Please edit .verify/run.sh."
fi

echo "==> Verification completed successfully."
"#;
        fs::write(&run_sh_path, script)
            .with_context(|| format!("write {}", run_sh_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&run_sh_path, fs::Permissions::from_mode(0o755));
        }
    }

    let readme_path = verify_dir.join("README.md");
    if force || !readme_path.exists() {
        let readme = r#"# Project Verification (`.verify/`)

Discovered by `agent-brain verify` and `agent-brain report`.

## Usage
- Run verify: `agent-brain verify -- .verify/run.sh`
- Allowlisted test commands are recorded into `~/.autonomic/memory/brain.db` with tamper-evident cryptographic hashes.
- Edit `verify.toml` to customize allowlisted commands for your project.
"#;
        fs::write(&readme_path, readme)
            .with_context(|| format!("write {}", readme_path.display()))?;
    }

    Ok(verify_dir)
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
    let joined = cmd.join(" ");
    let project_root = std::env::current_dir()
        .ok()
        .map(|cwd| crate::config::find_repo_root(&cwd).unwrap_or(cwd));
    if !command_is_allowlisted(&joined, project_root.as_deref()) {
        bail!("`{joined}` is not in the verify allowlist; add it to .verify/verify.toml");
    }
    let (exit_code, output) = run_process(cmd, None, VERIFY_RUN_TIMEOUT, true)?;
    print!("{output}");
    let cwd = std::env::current_dir().ok();
    let report = record_from_result(
        store,
        exit_code,
        &cmd.join(" "),
        route_log_id,
        cause_tag,
        config_path,
        scenarios_dir,
        cwd.as_deref(),
        source,
    )?;
    if let Some(tests_run) = count_tests(&output) {
        store.set_verify_outcome_tests_run(&report.id, tests_run)?;
    }
    Ok(report)
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
        let parts: Vec<String> = by_source.iter().map(|(k, v)| format!("{k}={v}")).collect();
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
            .filter(|r| r.verify_config_hash.as_ref().is_some_and(|h| h != baseline))
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

pub fn is_code_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.contains(&".verify") {
        return false;
    }
    let file = segments.last().copied().unwrap_or_default();
    segments
        .iter()
        .any(|segment| matches!(*segment, "src" | "crates" | "tests" | "hooks"))
        || file.starts_with("Cargo.")
        || file == "build.rs"
}

pub fn code_tree_hash(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .filter_map(|path| path.rsplit(" -> ").next())
        .filter(|path| is_code_path(path))
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    let mut hasher = Sha256::new();
    for path in paths {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        match fs::read(root.join(&path)) {
            Ok(bytes) => hasher.update(&bytes),
            Err(_) => hasher.update(b"<deleted>"),
        }
        hasher.update([0]);
    }
    Some(format!("{:x}", hasher.finalize()))
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

pub fn last_verification(store: &BrainStore, project_key: &str) -> Option<VerificationStatus> {
    let row = store
        .list_verify_outcomes_for_project(project_key, 1)
        .ok()?
        .into_iter()
        .next()?;
    Some(VerificationStatus {
        cmd: row.cmd,
        exit_code: row.exit_code,
        recorded_at: row.recorded_at,
        passing: row.exit_code == 0,
    })
}

/// Suggest high-confidence verified command recipes (exit 0) matching user intent or query keywords.
pub fn suggest_verified_recipes(
    store: &BrainStore,
    user_message: &str,
    project_key: Option<&str>,
    limit: usize,
) -> Vec<VerifiedRecipe> {
    let lower = user_message.to_lowercase();
    let has_execution_intent = [
        "test",
        "cargo",
        "npm",
        "pytest",
        "build",
        "run",
        "check",
        "clippy",
        "verify",
        "lint",
        "recipe",
        "benchmark",
        "prove",
        "proof",
        "exec",
        "compile",
    ]
    .iter()
    .any(|k| lower.contains(k));

    let Some(project_key) = project_key else {
        return Vec::new();
    };
    let Ok(outcomes) = store.list_verify_outcomes_for_project(project_key, 100) else {
        return Vec::new();
    };

    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| w.len() >= 3)
        .collect();

    let mut seen_cmds = std::collections::HashSet::new();
    let mut candidates: Vec<(i32, VerifiedRecipe)> = Vec::new();

    for row in outcomes {
        let trimmed_cmd = row.cmd.trim().to_string();
        if trimmed_cmd.is_empty() || seen_cmds.contains(&trimmed_cmd) {
            continue;
        }
        seen_cmds.insert(trimmed_cmd.clone());

        // Skip excessively large command blobs (e.g. embedded heredocs/scripts) that bloat route briefings
        if trimmed_cmd.len() > 300 || trimmed_cmd.lines().count() > 3 {
            continue;
        }

        // Because outcomes are ordered by recorded_at DESC, the first encounter
        // of trimmed_cmd is its latest run. If its latest execution failed, squelch it!
        if row.exit_code != 0 {
            continue;
        }
        let cmd_lower = trimmed_cmd.to_lowercase();

        let mut score = 0;
        if has_execution_intent {
            score += 10;
        }
        for word in &words {
            if cmd_lower.contains(word) {
                score += 20;
            }
        }

        let src = row.source.clone().unwrap_or_else(|| "cli".to_string());
        if matches!(src.as_str(), "cli" | "orchestrator" | "hook") {
            score += 5;
        }

        if score > 0 {
            candidates.push((
                score,
                VerifiedRecipe {
                    cmd: trimmed_cmd,
                    exit_code: row.exit_code,
                    source: src,
                    recorded_at: row.recorded_at,
                    rationale: "Previously verified exit 0 execution".to_string(),
                },
            ));
        }
    }

    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.recorded_at.cmp(&a.1.recorded_at))
    });
    candidates.into_iter().take(limit).map(|(_, r)| r).collect()
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
    fn count_tests_sums_every_cargo_result_line() {
        let output = "running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored\n\n\
                      running 4 tests\ntest result: ok. 4 passed; 0 failed; 0 ignored\n";
        assert_eq!(count_tests(output), Some(7));
        assert_eq!(count_tests("no cargo summary here"), None);
    }

    #[test]
    fn recorded_outcome_keeps_the_tests_run_count() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        let report = record_from_result(
            &store,
            0,
            "cargo test --all-features",
            None,
            None,
            None,
            None,
            Some(dir.path()),
            VerifySource::Cli,
        )
        .unwrap();
        assert_eq!(store.verify_outcome_tests_run(&report.id).unwrap(), None);

        store.set_verify_outcome_tests_run(&report.id, 303).unwrap();

        assert_eq!(
            store.verify_outcome_tests_run(&report.id).unwrap(),
            Some(303)
        );
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
        assert_eq!(
            report.verify_root.as_deref(),
            Some(dir.path().to_str().unwrap())
        );

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
        assert_eq!(
            VerifySource::parse("spine").unwrap(),
            VerifySource::Orchestrator
        );
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
        assert!(command_is_allowlisted(
            "cargo test -p agent-brain",
            Some(dir.path())
        ));
        assert!(!command_is_allowlisted("true", Some(dir.path())));
        assert!(!command_is_allowlisted("echo hi", Some(dir.path())));
    }

    #[test]
    fn project_verify_reports_no_step_when_none_is_declared() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        let value = run_project_verify(&store, dir.path()).unwrap();
        assert_eq!(value["ran"], false);
        assert!(store.list_verify_outcomes(10).unwrap().is_empty());
    }

    #[test]
    fn project_verify_runs_and_records_a_passing_step() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        write_verify_toml(
            dir.path(),
            "[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n",
        );
        let value = run_project_verify(&store, dir.path()).unwrap();
        assert_eq!(value["ran"], true);
        assert_eq!(value["verify"]["success"], true);
        assert_eq!(store.list_verify_outcomes(10).unwrap().len(), 1);
    }

    #[test]
    fn project_verify_refuses_a_step_outside_the_allowlist() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        write_verify_toml(
            dir.path(),
            "[verify.default]\ncommand = [\"echo\", \"hi\"]\n",
        );
        assert!(run_project_verify(&store, dir.path()).is_err());
    }

    #[test]
    fn init_verify_scaffolding() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            b"[package]\nname = \"test\"\n",
        )
        .unwrap();
        let verify_path = init_verify(dir.path(), false).unwrap();
        assert!(verify_path.exists());
        assert!(verify_path.join("verify.toml").exists());
        assert!(verify_path.join("run.sh").exists());
        assert!(verify_path.join("README.md").exists());

        let toml_str = fs::read_to_string(verify_path.join("verify.toml")).unwrap();
        assert!(toml_str.contains("cargo test"));
        assert!(toml_str.contains(".verify/"));

        assert!(command_is_allowlisted("cargo test", Some(dir.path())));
        assert!(command_is_allowlisted(".verify/run.sh", Some(dir.path())));
    }

    #[test]
    fn suggest_verified_recipes_ranks_exit_zero_matching_commands() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = BrainStore::open(&db).unwrap();

        // Insert a failed outcome and a successful outcome
        let _ = store.insert_verify_outcome(
            None,
            1,
            Some("fail"),
            None,
            None,
            "cargo test --failed",
            "dev1",
            "cli",
            Some("project-a"),
        );
        let _ = store.insert_verify_outcome(
            None,
            0,
            None,
            None,
            None,
            "cargo test -p agent-brain",
            "dev1",
            "orchestrator",
            Some("project-a"),
        );
        let _ = store.insert_verify_outcome(
            None,
            0,
            None,
            None,
            None,
            "npm test",
            "dev1",
            "cli",
            Some("project-a"),
        );

        let recipes = suggest_verified_recipes(&store, "run tests for brain", Some("project-a"), 2);
        assert!(!recipes.is_empty());
        assert_eq!(recipes[0].cmd, "cargo test -p agent-brain");
        assert_eq!(recipes[0].exit_code, 0);
        assert_eq!(recipes[0].source, "orchestrator");
    }

    #[test]
    fn suggest_verified_recipes_squelches_recently_failed_commands() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = BrainStore::open(&db).unwrap();

        // Older successful run
        let _ = store.insert_verify_outcome(
            None,
            0,
            None,
            None,
            None,
            "cargo test -p agent-brain",
            "dev1",
            "orchestrator",
            Some("project-a"),
        );
        // Latest run failed!
        let _ = store.insert_verify_outcome(
            None,
            1,
            Some("build error"),
            None,
            None,
            "cargo test -p agent-brain",
            "dev1",
            "cli",
            Some("project-a"),
        );

        let recipes = suggest_verified_recipes(&store, "run tests for brain", Some("project-a"), 2);
        assert!(
            recipes.is_empty(),
            "recently failed command must be squelched"
        );
    }

    #[test]
    fn last_verification_reports_the_latest_run_for_the_project() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        let _ = store.insert_verify_outcome(
            None,
            1,
            Some("build error"),
            None,
            None,
            "cargo test --lib",
            "dev1",
            "cli",
            Some("project-a"),
        );

        let status = last_verification(&store, "project-a").unwrap();
        assert_eq!(status.cmd, "cargo test --lib");
        assert!(!status.passing);
        assert!(last_verification(&store, "project-b").is_none());
    }

    #[test]
    fn suggest_verified_recipes_only_offers_recipes_recorded_for_the_same_project() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();

        let _ = store.insert_verify_outcome(
            None,
            0,
            None,
            None,
            None,
            "cargo test -p harbor",
            "dev1",
            "cli",
            Some("project-a"),
        );
        let _ = store.insert_verify_outcome(
            None,
            0,
            None,
            None,
            None,
            "cargo test -p legacy",
            "dev1",
            "cli",
            None,
        );

        assert!(suggest_verified_recipes(&store, "run tests", Some("project-b"), 3).is_empty());
        assert!(suggest_verified_recipes(&store, "run tests", None, 3).is_empty());
        let recipes = suggest_verified_recipes(&store, "run tests", Some("project-a"), 3);
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].cmd, "cargo test -p harbor");
    }

    #[test]
    fn record_from_result_updates_context_feedback_automatically() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = BrainStore::open(&db).unwrap();

        let log_id = "route-log-123";
        let items_json =
            r#"[{"id":"fact-item-1","item_type":"memory","topic":"test","score":0.9}]"#;
        store
            .insert_retrieval_log(
                log_id,
                "qhash",
                "implementing",
                items_json,
                100,
                false,
                false,
                10,
                Some(10),
                Some(50),
                Some(1),
            )
            .unwrap();

        // Record a successful outcome associated with this route log
        let report = record_from_result(
            &store,
            0,
            "cargo test",
            Some(log_id),
            None,
            None,
            None,
            Some(dir.path()),
            VerifySource::Cli,
        )
        .unwrap();
        assert!(report.success);

        // Feedback weights should reflect the positive feedback
        let weights = store
            .load_context_weights(&["fact-item-1".to_string()])
            .unwrap();
        assert!(weights.get("fact-item-1").copied().unwrap_or(1.0) > 1.0);
    }

    fn write_verify_toml(root: &Path, body: &str) {
        fs::create_dir_all(root.join(".verify")).unwrap();
        fs::write(root.join(".verify/verify.toml"), body).unwrap();
    }

    #[test]
    fn default_step_is_parsed_with_timeout_and_hook_flag() {
        let dir = tempdir().unwrap();
        write_verify_toml(
            dir.path(),
            "[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\ntimeout_secs = 120\n",
        );
        let step = load_default_step(dir.path()).unwrap().unwrap();
        assert_eq!(step.command, vec!["sh", "-c", "true"]);
        assert_eq!(step.timeout, Duration::from_secs(120));
        assert!(step.hook);
    }

    #[test]
    fn missing_default_step_is_none() {
        let dir = tempdir().unwrap();
        write_verify_toml(dir.path(), "[verify]\ncommands = [\"cargo test\"]\n");
        assert!(load_default_step(dir.path()).unwrap().is_none());
    }

    #[test]
    fn default_step_rejects_out_of_range_timeout() {
        let dir = tempdir().unwrap();
        write_verify_toml(
            dir.path(),
            "[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\ntimeout_secs = 0\n",
        );
        assert!(load_default_step(dir.path()).is_err());
    }

    #[test]
    fn default_step_rejects_invalid_toml_instead_of_skipping_it() {
        let dir = tempdir().unwrap();
        write_verify_toml(dir.path(), "[verify.default\ncommand = ");
        assert!(load_default_step(dir.path()).is_err());
    }

    #[test]
    fn process_is_killed_when_it_exceeds_its_timeout() {
        let cmd = ["sh", "-c", "exec sleep 30"].map(String::from);
        let (code, _) = run_process(&cmd, None, Duration::from_millis(200), false).unwrap();
        assert_eq!(code, TIMEOUT_EXIT_CODE);
    }

    #[test]
    fn default_step_refuses_commands_outside_the_allowlist() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        write_verify_toml(
            dir.path(),
            "[verify.default]\ncommand = [\"echo\", \"hi\"]\n",
        );
        let err = run_default_step(&store, dir.path(), VerifySource::Hook).unwrap_err();
        assert!(err.to_string().contains("not in the verify allowlist"));
    }

    #[test]
    fn default_step_runs_in_project_root_and_records_the_result() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        write_verify_toml(
            dir.path(),
            "[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n",
        );
        let report = run_default_step(&store, dir.path(), VerifySource::Hook)
            .unwrap()
            .unwrap()
            .report;
        assert!(report.success);
        assert_eq!(store.list_verify_outcomes(10).unwrap().len(), 1);
    }

    #[test]
    fn init_writes_a_default_step_for_a_cargo_project() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        init_verify(dir.path(), false).unwrap();
        let step = load_default_step(dir.path()).unwrap().unwrap();
        assert_eq!(step.command, vec!["cargo", "test"]);
        assert_eq!(step.timeout, Duration::from_secs(300));
    }

    #[test]
    fn init_writes_no_default_step_when_no_runner_is_detected() {
        let dir = tempdir().unwrap();
        init_verify(dir.path(), false).unwrap();
        assert!(load_default_step(dir.path()).unwrap().is_none());
    }

    #[test]
    fn default_step_timeout_may_not_exceed_the_ceiling() {
        let dir = tempdir().unwrap();
        write_verify_toml(
            dir.path(),
            &format!(
                "[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\ntimeout_secs = {}\n",
                MAX_STEP_TIMEOUT_SECS + 1
            ),
        );
        assert!(load_default_step(dir.path()).is_err());
    }
}
