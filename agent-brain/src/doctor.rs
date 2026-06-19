use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureStatus {
    Ok,
    Missing,
    Invalid,
    #[cfg(target_os = "macos")]
    LinkerSigned,
}

pub fn run(fix: bool) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().context("current_exe")?;
    let config = crate::config::Config::load()?;
    let home = dirs::home_dir().context("home dir")?;
    let mcp_path = home.join(".cursor/mcp.json");
    let hooks_path = home.join(".cursor/hooks.json");
    let briefing_path = config.home.join("logs/last-route.md");

    let mcp_binary = mcp_binary_path(&mcp_path)?;

    println!("agent-brain doctor\n");
    println!("  version (this binary): {version}");
    println!("  binary path:           {}", exe.display());

    let mut ok = true;

    if let Some(cmd) = &mcp_binary {
        println!("  mcp.json command:      {}", cmd.display());
        if paths_same(&exe, cmd) {
            println!("  mcp path:              OK");
        } else if fs::canonicalize(cmd).ok() == fs::canonicalize(&exe).ok() {
            println!("  mcp path:              OK (same binary, different path spelling)");
        } else {
            println!("  mcp path:              MISMATCH");
            ok = false;
            if fix {
                println!("  fixing:                agent-brain install --global");
                crate::install::configure_cursor(true, &exe, false)?;
                let _ = crate::host_install::install_host(
                    crate::host_install::HostTarget::All,
                    &exe,
                    true,
                );
                println!("  mcp path:              realigned to {}", exe.display());
                ok = true;
            } else {
                println!("                         run: agent-brain doctor --fix");
            }
        }
    } else if mcp_path.exists() {
        println!("  mcp.json:              missing agent-brain entry");
        ok = false;
    } else {
        println!("  mcp.json:              not found — run: agent-brain install --global");
        ok = false;
    }

    if hooks_path.exists() {
        let raw = fs::read_to_string(&hooks_path)?;
        if raw.contains("route_gate.py") {
            println!("  hooks:                 OK (route_gate installed)");
        } else {
            println!("  hooks:                 route_gate not found");
            ok = false;
            if fix {
                crate::install::configure_cursor(true, &exe, false)?;
                println!("  hooks:                 reinstalled");
                ok = true;
            }
        }
    } else {
        println!("  hooks:                 not found");
        ok = false;
    }

    let claude_settings = home.join(".claude/settings.json");
    let claude_hooks = if crate::host_hooks::claude_hooks_need_refresh(&claude_settings) {
        if claude_settings.is_file() {
            "stale (reinstall)"
        } else {
            crate::host_hooks::hooks_status(&claude_settings, "route_gate.py")
        }
    } else {
        "OK"
    };
    println!("  claude-code hooks:     {claude_hooks}");
    if claude_hooks != "OK" {
        ok = false;
        if fix {
            crate::host_hooks::install_claude_code_hooks(true, false)?;
            println!("  claude-code hooks:     reinstalled");
            ok = true;
        }
    }

    let mut sign_targets = vec![exe.clone()];
    if let Some(cmd) = mcp_binary.clone() {
        if !paths_same(&exe, &cmd) {
            sign_targets.push(cmd);
        }
    }

    for path in unique_existing_paths(sign_targets) {
        let status = assess_signature(&path);
        let gate = gatekeeper_allows(&path);
        let quarantine = macos_has_quarantine_attrs(&path);
        let gate_note = if gate {
            " · Gatekeeper OK"
        } else if status == SignatureStatus::Ok {
            " · Gatekeeper rejected (expected for adhoc local builds)"
        } else {
            " · Gatekeeper REJECTED"
        };
        println!(
            "  codesign {}:        {:?}{}",
            path.display(),
            status,
            gate_note
        );
        if quarantine {
            println!(
                "  quarantine xattrs:     present on {} (Cursor may SIGKILL until cleared)",
                path.display()
            );
            ok = false;
        }
        if status != SignatureStatus::Ok || quarantine {
            ok = false;
            if fix {
                adhoc_sign(&path).with_context(|| format!("sign {}", path.display()))?;
                let after = assess_signature(&path);
                let gate_after = gatekeeper_allows(&path);
                let after_gate_note = if gate_after {
                    " · Gatekeeper OK"
                } else if after == SignatureStatus::Ok {
                    " · Gatekeeper rejected (expected for adhoc local builds)"
                } else {
                    " · Gatekeeper still rejected"
                };
                println!(
                    "  signed {}:           {:?}{}",
                    path.display(),
                    after,
                    after_gate_note
                );
                if after == SignatureStatus::Ok && !macos_has_quarantine_attrs(&path) {
                    ok = true;
                }
            } else if quarantine {
                println!(
                    "                         run: xattr -cr {} && codesign --force --sign - {}",
                    path.display(),
                    path.display()
                );
                println!("                         or: agent-brain doctor --fix");
            } else if status != SignatureStatus::Ok {
                println!("                         run: agent-brain doctor --fix");
            }
        }
    }

    if briefing_path.is_file() {
        println!("  last route briefing:   {}", briefing_path.display());
    } else {
        println!("  last route briefing:   not yet (appears after first route_task)");
    }

    print_other_hosts(&home);

    let serve = crate::serve_meta::assess(&config.home, mcp_binary.as_deref());
    if let Some(meta) = &serve.meta {
        let alive = if serve.process_alive { "running" } else { "not running" };
        println!(
            "  last serve:            v{} pid {} ({})",
            meta.version, meta.pid, alive
        );
    } else {
        println!("  last serve:            unknown (starts after next MCP connect)");
    }
    if let Some(disk) = &serve.disk_version {
        println!("  binary on disk:        {disk}");
    }
    if serve.stale {
        println!("  serve stale:           YES — running process is older than binary on disk");
        ok = false;
        if fix {
            println!("  fixing:                agent-brain install --global --reload");
            crate::install::configure_cursor(true, &exe, false)?;
            println!("  reload nudge:          mcp.json refreshed (toggle MCP if still stale)");
        } else {
            println!("                         run: agent-brain install --global --reload");
            println!("                         or toggle agent-brain under Settings → MCP");
        }
    } else if serve.meta.is_some() && serve.process_alive {
        println!("  serve stale:           no");
    }

    println!();
    println!("Tips:");
    println!("  • agent-brain briefing — readable route + estimated token savings vs full index");
    println!("  • agent-brain stats — index size, savings, latency, adoption milestones");
    println!("  • agent-brain dashboard --open — local HTML value dashboard (screenshot-friendly ROI)");
    println!("  • agent-brain onboarding — 5-minute getting started checklist");
    println!("  • agent-brain install --all --global — MCP + instructions for Cursor, OpenCode, Claude Code, VS Code, Codex, Gemini, Antigravity");
    println!("  • Claude Code, Codex, Gemini, Antigravity, OpenCode: route gate hooks installed via install --<host> [--global]");
    println!("  • Cursor has the strongest host-tool gate (hooks on Read/Shell); other hosts gate agent-brain MCP tools until route_task");
    println!("  • Background auto-update during serve can exec a new binary after idle (see config auto_update.mcp.restart_after_update)");
    println!("  • macOS: linker-signed binaries are killed by taskgated — doctor --fix adhoc re-signs");
    println!("  • macOS: browser/curl downloads add quarantine xattrs — xattr -cr + adhoc codesign before Cursor MCP");
    println!("  • spctl may reject adhoc local builds; that is OK if codesign shows adhoc, not linker-signed");

    if !ok {
        if fix {
            println!();
            println!("doctor --fix finished; review any remaining issues above.");
            bail!("doctor --fix completed with remaining issues");
        }
        println!();
        println!("Self-heal:  agent-brain doctor --fix");
        println!("             Re-aligns MCP config, Cursor hooks, and macOS codesign/quarantine.");
        println!("             Restart Cursor after fix if hooks or MCP were stale.");
        std::process::exit(1);
    }
    if fix {
        if let Ok((indexed, sessions)) = (|| {
            let engine = crate::engine::Engine::new(config.clone())?;
            engine.post_install_warmup()
        })() {
            println!(
                "  post-fix index:        {indexed} items · {sessions} session digests ingested"
            );
        }
    }
    if let Ok(store) = crate::db::store::BrainStore::open(&config.db_path) {
        if let Ok(stats) = crate::stats::collect(&store, &config, 7) {
            println!();
            println!("Stats: {}", crate::stats::format_summary_line(&stats));
        }
    }
    crate::onboarding::print_onboarding(&config.home, briefing_path.is_file());
    Ok(())
}

fn print_other_hosts(home: &Path) {
    let opencode = home.join(".config/opencode/opencode.json");
    let codex = home.join(".codex/config.toml");
    let codex_hooks = home.join(".codex/hooks.json");
    let claude = home.join(".claude.json");
    let claude_settings = home.join(".claude/settings.json");
    let gemini = home.join(".gemini/settings.json");
    let antigravity = home.join(".gemini/antigravity/mcp_config.json");
    let antigravity_shared = home.join(".gemini/config/mcp_config.json");
    println!("  opencode (global):     {}", host_mcp_status(&opencode, "mcp", "agent-brain"));
    println!("  codex (global):        {}", codex_mcp_status(&codex));
    println!(
        "  codex hooks:           {}",
        crate::host_hooks::hooks_status(&codex_hooks, "route_gate.py")
    );
    println!("  claude-code (global):  {}", host_mcp_status(&claude, "mcpServers", "agent-brain"));
    println!(
        "  claude hooks:          {}",
        crate::host_hooks::hooks_status(&claude_settings, "route_gate.py")
    );
    println!("  gemini (global):       {}", host_mcp_status(&gemini, "mcpServers", "agent-brain"));
    println!(
        "  gemini hooks:          {}",
        crate::host_hooks::hooks_status(&gemini, "route_gate.py")
    );
    println!(
        "  antigravity (global):  {}",
        antigravity_host_status(&antigravity, &antigravity_shared)
    );
}

fn antigravity_host_status(primary: &Path, shared: &Path) -> &'static str {
    let primary_status = host_mcp_status(primary, "mcpServers", "agent-brain");
    if primary_status == "OK" {
        return "OK";
    }
    host_mcp_status(shared, "mcpServers", "agent-brain")
}

fn codex_mcp_status(path: &Path) -> &'static str {
    if !path.is_file() {
        return "not configured";
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return "unreadable";
    };
    if raw.contains("[mcp_servers.agent-brain]") {
        "OK"
    } else {
        "missing agent-brain entry"
    }
}

fn host_mcp_status(path: &Path, servers_key: &str, server_name: &str) -> &'static str {
    if !path.is_file() {
        return "not configured";
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return "unreadable";
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return "invalid json";
    };
    if root
        .pointer(&format!("/{servers_key}/{server_name}"))
        .is_some()
    {
        "OK"
    } else {
        "missing agent-brain entry"
    }
}

fn mcp_binary_path(mcp_path: &Path) -> Result<Option<PathBuf>> {
    if !mcp_path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(mcp_path)?;
    let root: serde_json::Value = serde_json::from_str(&raw)?;
    Ok(root
        .pointer("/mcpServers/agent-brain/command")
        .and_then(|v| v.as_str())
        .map(PathBuf::from))
}

fn paths_same(a: &Path, b: &Path) -> bool {
    a == b || fs::canonicalize(a).ok() == fs::canonicalize(b).ok()
}

fn unique_existing_paths(entries: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for candidate in entries {
        if !candidate.is_file() {
            continue;
        }
        let dup = out.iter().any(|existing| {
            paths_same(existing.as_path(), candidate.as_path())
        });
        if !dup {
            out.push(candidate);
        }
    }
    out
}

pub fn assess_signature(path: &Path) -> SignatureStatus {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("codesign")
            .args(["-dv", "--verbose=2", &path.display().to_string()])
            .output();
        let Ok(output) = output else {
            return SignatureStatus::Missing;
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            return SignatureStatus::Invalid;
        }
        if stderr.contains("linker-signed") {
            return SignatureStatus::LinkerSigned;
        }
        if stderr.contains("Signature=adhoc") || stderr.contains("Signature=Apple") {
            return SignatureStatus::Ok;
        }
        if stderr.contains("code object is not signed") {
            return SignatureStatus::Invalid;
        }
        SignatureStatus::Ok
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        SignatureStatus::Ok
    }
}

pub fn gatekeeper_allows(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("spctl")
            .args([
                "-a",
                "-vv",
                "-t",
                "execute",
                &path.display().to_string(),
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        true
    }
}

/// Download quarantine blocks execution when Cursor (taskgated) spawns the MCP binary.
pub fn macos_has_quarantine_attrs(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("xattr")
            .arg(&path.display().to_string())
            .output()
            .map(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                out.contains("com.apple.quarantine")
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

/// Ad-hoc sign + clear quarantine xattrs (required after copy on macOS).
pub fn adhoc_sign(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("xattr")
            .args(["-cr", &path.display().to_string()])
            .status()
            .context("xattr -cr")?;
        let status = Command::new("codesign")
            .args([
                "--force",
                "--sign",
                "-",
                &path.display().to_string(),
            ])
            .status()
            .context("codesign")?;
        if !status.success() {
            bail!("codesign failed for {}", path.display());
        }
        let verify = Command::new("codesign")
            .args(["--verify", "--verbose", &path.display().to_string()])
            .status()
            .context("codesign --verify")?;
        if !verify.success() {
            bail!("codesign verify failed for {}", path.display());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(())
    }
}
