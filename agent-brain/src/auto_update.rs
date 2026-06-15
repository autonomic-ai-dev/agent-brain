use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::expand_home;
use crate::engine::Engine;
use crate::install;
use crate::mcp_activity::McpActivity;
use crate::packages::{self, PackageRecord, PackageRegistry};
use crate::settings::{AgentBrainSettings, AutoUpdateSettings, McpAutoUpdateSettings};

const STATE_FILE: &str = "auto_update_state.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoUpdateState {
    pub last_run_unix: i64,
    pub last_mcp_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoUpdateRunOptions {
    /// When true and MCP binary was updated, restart this process so Cursor loads the new build.
    pub restart_mcp_if_serving: bool,
}

impl AutoUpdateRunOptions {
    pub fn cli() -> Self {
        Self::default()
    }

    pub fn background_serve() -> Self {
        Self {
            restart_mcp_if_serving: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoUpdateReport {
    pub packages_updated: usize,
    pub mcp_updated: bool,
    pub mcp_version: Option<String>,
    pub reindexed: bool,
    pub mcp_restart_scheduled: bool,
}

impl AutoUpdateState {
    pub fn load(home: &Path) -> Self {
        let path = home.join(STATE_FILE);
        if !path.exists() {
            return Self::default();
        }
        fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let path = home.join(STATE_FILE);
        let pretty = serde_json::to_string_pretty(self)?;
        fs::write(path, format!("{pretty}\n")).context("write auto_update_state.json")
    }
}

pub fn run(
    engine: &Arc<Engine>,
    settings: &AgentBrainSettings,
    force: bool,
    options: AutoUpdateRunOptions,
) -> Result<AutoUpdateReport> {
    let cfg = &settings.auto_update;
    if !cfg.enabled {
        return Ok(AutoUpdateReport::default());
    }

    let home = &engine.config.home;
    let mut state = AutoUpdateState::load(home);
    if !force && !due_for_run(cfg, &state) {
        tracing::debug!(target: "agent_brain::auto_update", "skipped (interval not elapsed)");
        return Ok(AutoUpdateReport::default());
    }

    let mut report = AutoUpdateReport::default();
    let current_version = env!("CARGO_PKG_VERSION");

    if cfg.mcp.enabled {
        match update_mcp_binary(cfg, current_version)? {
            Some(version) => {
                report.mcp_updated = true;
                report.mcp_version = Some(version.clone());
                state.last_mcp_version = Some(version);
            }
            None => tracing::debug!(target: "agent_brain::auto_update", "mcp already current"),
        }
    }

    if cfg.packages.enabled {
        report.packages_updated = update_configured_packages(engine, cfg)?;
    }

    let will_restart = report.mcp_updated
        && options.restart_mcp_if_serving
        && cfg.mcp.restart_after_update
        && can_restart_in_place(&expand_home(&cfg.mcp.bin_path));

    if (report.mcp_updated || report.packages_updated > 0) && !will_restart {
        match engine.bootstrap(None) {
            Ok(n) => {
                report.reindexed = true;
                tracing::info!(
                    target: "agent_brain::auto_update",
                    items = n,
                    packages = report.packages_updated,
                    mcp = report.mcp_updated,
                    "reindexed after auto-update"
                );
            }
            Err(err) => tracing::warn!(error = %err, "auto-update reindex failed"),
        }
    } else if report.packages_updated > 0 && will_restart {
        tracing::info!(
            target: "agent_brain::auto_update",
            packages = report.packages_updated,
            "deferring reindex until MCP restart loads new binary"
        );
    }

    state.last_run_unix = chrono::Utc::now().timestamp();
    state.save(home)?;

    if will_restart {
        let bin_path = expand_home(&cfg.mcp.bin_path);
        let version = report.mcp_version.clone().unwrap_or_default();
        schedule_mcp_restart(
            bin_path,
            version,
            engine.mcp_activity.clone(),
            McpRestartPolicy::from(&cfg.mcp),
        );
        report.mcp_restart_scheduled = true;
    } else if report.mcp_updated {
        let bin_path = expand_home(&cfg.mcp.bin_path);
        eprintln!(
            "agent-brain: MCP binary updated at {}; restart or toggle MCP in Cursor to load it",
            bin_path.display()
        );
    }

    Ok(report)
}

fn due_for_run(cfg: &AutoUpdateSettings, state: &AutoUpdateState) -> bool {
    if state.last_run_unix == 0 {
        return true;
    }
    let elapsed = chrono::Utc::now().timestamp() - state.last_run_unix;
    elapsed >= (cfg.interval_hours as i64) * 3600
}

fn update_configured_packages(engine: &Arc<Engine>, cfg: &AutoUpdateSettings) -> Result<usize> {
    let registry = PackageRegistry::load(&engine.config.home)?;
    if registry.packages.is_empty() {
        return Ok(0);
    }

    let targets: Vec<PackageRecord> = if cfg.packages.names.is_empty() {
        registry.packages.clone()
    } else {
        cfg.packages
            .names
            .iter()
            .filter_map(|name| registry.get(name).cloned())
            .collect()
    };

    if targets.is_empty() {
        return Ok(0);
    }

    let before: Vec<_> = targets
        .iter()
        .map(|p| (p.name.clone(), p.commit.clone()))
        .collect();

    let mut updated = 0usize;
    for name in before.iter().map(|(n, _)| n) {
        let pkgs = packages::update_packages(&engine.config, Some(name))?;
        for pkg in pkgs {
            let prev = before.iter().find(|(n, _)| n == &pkg.name).map(|(_, c)| c.as_deref());
            if prev != Some(pkg.commit.as_deref()) {
                updated += 1;
                tracing::info!(
                    target: "agent_brain::auto_update",
                    package = %pkg.name,
                    commit = pkg.commit.as_deref().unwrap_or("-"),
                    "package updated"
                );
            }
        }
    }
    Ok(updated)
}

fn update_mcp_binary(cfg: &AutoUpdateSettings, current_version: &str) -> Result<Option<String>> {
    let latest = fetch_latest_release_tag(&cfg.mcp.repo)?;
    let latest_version = latest.trim_start_matches('v');
    if version_is_newer(current_version, latest_version) {
        tracing::info!(
            target: "agent_brain::auto_update",
            local = current_version,
            latest = latest_version,
            "local MCP binary is ahead of GitHub latest; skipping download"
        );
        return Ok(None);
    }
    if !version_is_newer(latest_version, current_version) {
        return Ok(None);
    }

    let target = detect_release_target().context("unsupported platform for mcp auto-update")?;
    let asset = release_artifact_name(&target);
    let url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        cfg.mcp.repo, latest, asset
    );

    let bin_path = expand_home(&cfg.mcp.bin_path);
    if let Some(parent) = bin_path.parent() {
        fs::create_dir_all(parent).context("create bin parent dir")?;
    }

    let tmp = bin_path.with_extension("download");
    curl_download(&url, &tmp)?;
    set_executable(&tmp)?;

    if bin_path.exists() {
        fs::remove_file(&bin_path).ok();
    }
    fs::rename(&tmp, &bin_path).with_context(|| format!("install {}", bin_path.display()))?;

    tracing::info!(
        target: "agent_brain::auto_update",
        from = current_version,
        to = latest_version,
        path = %bin_path.display(),
        "mcp binary updated"
    );

    if cfg.mcp.refresh_cursor {
        install::configure_cursor(true, &bin_path, true)
            .with_context(|| "refresh Cursor MCP/hooks after binary update")?;
    }

    eprintln!(
        "agent-brain: MCP binary updated to v{latest_version} at {}",
        bin_path.display()
    );

    Ok(Some(latest_version.to_string()))
}

const RESTART_ENV_KEYS: &[&str] = &[
    "AGENT_BRAIN_HOME",
    "AGENT_BRAIN_BOOTSTRAP_BG",
    "AGENT_BRAIN_PREWARM",
    "AGENT_BRAIN_ROUTE_HOOKS",
    "RUST_LOG",
];

#[derive(Debug, Clone, Copy)]
pub struct McpRestartPolicy {
    pub idle_secs: u64,
    pub max_wait_secs: u64,
    pub min_delay_secs: u64,
}

impl From<&McpAutoUpdateSettings> for McpRestartPolicy {
    fn from(cfg: &McpAutoUpdateSettings) -> Self {
        Self {
            idle_secs: cfg.restart_idle_secs.min(120),
            max_wait_secs: cfg.restart_max_wait_secs.clamp(30, 3600),
            min_delay_secs: cfg.restart_min_delay_secs.min(30),
        }
    }
}

/// Replace this MCP server process with the new binary (Unix) or exit so Cursor respawns it.
pub fn schedule_mcp_restart(
    bin_path: PathBuf,
    version: String,
    activity: Arc<McpActivity>,
    policy: McpRestartPolicy,
) {
    eprintln!(
        "agent-brain: scheduling MCP restart for v{version} after idle ({}s quiet, max wait {}s)",
        policy.idle_secs, policy.max_wait_secs
    );
    std::thread::spawn(move || {
        wait_for_safe_restart(&activity, policy);
        restart_mcp_process(&bin_path, &version);
    });
}

fn wait_for_safe_restart(activity: &McpActivity, policy: McpRestartPolicy) {
    std::thread::sleep(Duration::from_secs(policy.min_delay_secs));
    let started = std::time::Instant::now();
    loop {
        if activity.idle_for_secs(policy.idle_secs) {
            eprintln!(
                "agent-brain: MCP idle for {}s; restarting to load new binary",
                policy.idle_secs
            );
            return;
        }
        if started.elapsed() >= Duration::from_secs(policy.max_wait_secs) {
            eprintln!(
                "agent-brain: restart max wait {}s exceeded; restarting anyway",
                policy.max_wait_secs
            );
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

pub fn can_restart_in_place(bin_path: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    paths_equal(&current, bin_path)
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn restart_mcp_process(bin_path: &Path, version: &str) {
    tracing::info!(
        target: "agent_brain::auto_update",
        version = version,
        path = %bin_path.display(),
        "restarting MCP after binary update"
    );

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut cmd = Command::new(bin_path);
        cmd.arg("serve");
        for key in RESTART_ENV_KEYS {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        eprintln!("agent-brain: exec `serve` v{version}");
        let err = cmd.exec();
        eprintln!("agent-brain: exec failed ({err}); exiting so Cursor can respawn MCP");
    }

    #[cfg(not(unix))]
    {
        eprintln!("agent-brain: exiting so Cursor can respawn MCP v{version}");
    }

    std::process::exit(0);
}

pub fn should_schedule_mcp_restart(
    options: AutoUpdateRunOptions,
    cfg: &AutoUpdateSettings,
    mcp_updated: bool,
) -> bool {
    mcp_updated && options.restart_mcp_if_serving && cfg.mcp.restart_after_update
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
}

fn fetch_latest_release_tag(repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = curl_stdout(&["-fsSL", &url])?;
    let release: GhRelease = serde_json::from_str(&body).context("parse GitHub release JSON")?;
    Ok(release.tag_name)
}

fn curl_stdout(args: &[&str]) -> Result<String> {
    let output = Command::new("curl")
        .args(args)
        .output()
        .context("spawn curl")?;
    if !output.status.success() {
        bail!(
            "curl failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn curl_download(url: &str, dest: &Path) -> Result<()> {
    let dest_str = dest.to_string_lossy();
    let status = Command::new("curl")
        .args(["-fsSL", url, "-o", dest_str.as_ref()])
        .status()
        .context("spawn curl download")?;
    if !status.success() {
        bail!("curl download failed for {url}");
    }
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

pub fn detect_release_target() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

fn release_artifact_name(target: &str) -> String {
    if target.contains("windows") {
        format!("agent-brain-{target}.exe")
    } else {
        format!("agent-brain-{target}")
    }
}

pub fn version_is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn parse_version(raw: &str) -> Vec<u32> {
    raw.trim()
        .trim_start_matches('v')
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_scheduled_only_when_serving() {
        let cfg = AutoUpdateSettings::default();
        assert!(should_schedule_mcp_restart(
            AutoUpdateRunOptions::background_serve(),
            &cfg,
            true
        ));
        assert!(!should_schedule_mcp_restart(AutoUpdateRunOptions::cli(), &cfg, true));
    }

    #[test]
    fn version_compare_orders_semver_parts() {
        assert!(version_is_newer("0.3.12", "0.3.11"));
        assert!(!version_is_newer("0.3.11", "0.3.12"));
        assert!(version_is_newer("0.4.0", "0.3.99"));
    }

    #[test]
    fn release_artifact_names_match_install_script() {
        assert_eq!(
            release_artifact_name("aarch64-apple-darwin"),
            "agent-brain-aarch64-apple-darwin"
        );
        assert_eq!(
            release_artifact_name("x86_64-pc-windows-msvc"),
            "agent-brain-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn interval_gate_respects_state() {
        let cfg = AutoUpdateSettings {
            enabled: true,
            interval_hours: 24,
            ..AutoUpdateSettings::default()
        };
        let state = AutoUpdateState {
            last_run_unix: chrono::Utc::now().timestamp(),
            ..Default::default()
        };
        assert!(!due_for_run(&cfg, &state));
    }
}
