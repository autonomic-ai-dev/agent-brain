use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::host_install::{self, HostTarget};

pub fn run(target: HostTarget, print_only: bool, reload: bool) -> Result<()> {
    let exe = std::env::current_exe().context("resolve agent-brain binary path")?;
    let mcp_exe = if print_only {
        exe.clone()
    } else {
        ensure_mcp_runnable_binary(&exe, false)?
    };
    let snippet = mcp_server_entry(&mcp_exe);

    if print_only {
        match target {
            HostTarget::VsCode { .. } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "servers": { "agent-brain": host_install::vscode_server_entry_public(&exe) }
                    }))?
                );
            }
            HostTarget::OpenCode { .. } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "mcp": { "agent-brain": host_install::opencode_server_entry_public(&exe) }
                    }))?
                );
            }
            HostTarget::Codex { .. } => {
                println!("{}", host_install::codex_mcp_toml_block(&exe));
            }
            _ => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({ "mcpServers": { "agent-brain": snippet } })
                    )?
                );
            }
        }
        return Ok(());
    }

    if matches!(target, HostTarget::Cursor { global: true }) || reload {
        if reload && !matches!(target, HostTarget::Cursor { .. } | HostTarget::All) {
            eprintln!("Note: --reload only applies to Cursor MCP config.");
        }
    }

    let paths = host_install::install_host(target, &mcp_exe, false)?;
    println!("agent-brain MCP configured for {}", target.label());
    println!("Binary: {}", exe.display());
    if mcp_exe != exe {
        println!("MCP spawn path: {} (signed copy for Cursor)", mcp_exe.display());
    }
    for path in paths {
        println!("  {}", path.display());
    }
    println!();

    match target {
        HostTarget::Cursor { global: true } if reload => {
            println!("Reload nudge: refreshed Cursor mcp.json (AGENT_BRAIN_BUILD bumped).");
            println!("Toggle agent-brain under Settings → MCP if it does not reconnect.");
        }
        HostTarget::All if reload => {
            println!("Reload nudge: refreshed Cursor mcp.json (AGENT_BRAIN_BUILD bumped).");
            println!("Toggle agent-brain under Settings → MCP if it does not reconnect.");
        }
        HostTarget::Cursor { global: true } => {
            print_cursor_next_steps();
        }
        HostTarget::All => print_cursor_next_steps(),
        _ => {}
    }

    #[cfg(target_os = "macos")]
    {
        if mcp_exe == exe {
            if let Err(err) = crate::doctor::adhoc_sign(&exe) {
                eprintln!("Warning: adhoc codesign failed: {err}");
            }
        }
    }

    if let Err(err) = run_post_install_warmup(false) {
        eprintln!("Warning: post-install index/session ingest: {err}");
    }

    Ok(())
}

/// Index skills/rules and ingest Cursor/OpenCode/Codex/Gemini session digests into brain.db.
pub fn run_post_install_warmup(quiet: bool) -> Result<()> {
    let config = crate::config::Config::load()?;
    config.ensure_dirs()?;
    let engine = crate::engine::Engine::new(config)?;
    let (indexed, sessions) = engine.post_install_warmup()?;
    if !quiet {
        println!(
            "Post-install brain: indexed {indexed} items · ingested {sessions} session digests (cursor/opencode/codex/gemini)"
        );
    }
    Ok(())
}

fn print_cursor_next_steps() {
    println!("Cursor next steps:");
    println!("  1. Paste User Rules from ~/.agent_brain/cursor-user-rules.mdc");
    println!("     Optional mode: ~/.agent_brain/cursor-agent-brain-mode.mdc");
    println!("     (Settings → Rules, Memories, and Commands → User Rules)");
    println!("  2. Restart Cursor or toggle agent-brain under Settings → MCP");
    println!("  3. Confirm hooks under Settings → Hooks (route_task gate)");
    println!("  4. Other hosts: agent-brain mode paths");
    println!("  5. After rebuilds or brew upgrade: agent-brain install --global --reload");
    println!();
    println!("Other hosts: agent-brain install --claude-desktop | --vscode | --claude-code | --opencode | --codex | --gemini | --antigravity [--global] | --all");
}

/// macOS MCP path: Homebrew Cellar blocks xattr/codesign — copy to ~/.local/bin (user-writable).
pub fn ensure_mcp_runnable_binary(exe: &Path, quiet: bool) -> Result<PathBuf> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = quiet;
        return Ok(exe.to_path_buf());
    }

    #[cfg(target_os = "macos")]
    {
        let canonical = fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
        if !needs_mcp_binary_copy(&canonical) {
            if let Err(err) = crate::doctor::adhoc_sign(&canonical) {
                if !quiet {
                    eprintln!("Warning: adhoc codesign on {}: {err}", canonical.display());
                }
            }
            return Ok(canonical);
        }

        let dest = dirs::home_dir()
            .context("home directory")?
            .join(".local/bin/agent-brain");
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&canonical, &dest)
            .with_context(|| format!("copy MCP binary to {}", dest.display()))?;
        crate::doctor::adhoc_sign(&dest).with_context(|| {
            format!(
                "sign MCP binary at {} (required for Cursor on macOS)",
                dest.display()
            )
        })?;
        if !quiet {
            println!(
                "MCP binary: {} (copy of {} — Homebrew/cellar paths cannot be re-signed in place)",
                dest.display(),
                canonical.display()
            );
        }
        Ok(dest)
    }
}

#[cfg(target_os = "macos")]
fn needs_mcp_binary_copy(path: &Path) -> bool {
    is_homebrew_managed(path) || crate::doctor::macos_has_quarantine_attrs(path)
}

#[cfg(not(target_os = "macos"))]
fn needs_mcp_binary_copy(_path: &Path) -> bool {
    false
}

pub fn is_homebrew_managed(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/Cellar/")
        || s.contains("/opt/homebrew/")
        || s.contains("/usr/local/Cellar/")
}

/// Merge MCP config and optionally refresh Cursor hooks/rule (used by install + auto-update).
pub fn configure_cursor(global: bool, exe: &Path, quiet: bool) -> Result<()> {
    let mcp_exe = ensure_mcp_runnable_binary(exe, quiet)?;
    let config_path = mcp_config_path(global)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let merged = merge_mcp_config(&config_path, mcp_server_entry(&mcp_exe))?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(&config_path, format!("{pretty}\n"))
        .with_context(|| format!("write MCP config to {}", config_path.display()))?;

    if global {
        install_cursor_hooks(quiet)?;
        install_cursor_permissions(quiet)?;
        install_cursor_user_rules_snippet(quiet)?;
        host_install::install_agent_brain_modes(true, quiet)?;
    } else {
        install_project_cursor_rules(quiet)?;
    }

    Ok(())
}

/// Cursor loads **project** rules from `<workspace>/.cursor/rules/*.mdc` only.
/// For global installs, write a User Rules snippet under ~/.agent_brain/.
pub fn install_cursor_user_rules_snippet(quiet: bool) -> Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let brain_home = home.join(".agent_brain");
    fs::create_dir_all(&brain_home)
        .with_context(|| format!("create {}", brain_home.display()))?;
    let path = brain_home.join("cursor-user-rules.mdc");
    fs::write(&path, CURSOR_RULE).with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!();
        println!("Cursor User Rules (paste manually — Cursor does not load ~/.cursor/rules/ globally):");
        println!("  File: {}", path.display());
        println!("  Cursor → Settings → Rules, Memories, and Commands → User Rules → paste file contents");
        println!("  Hooks in ~/.cursor/hooks.json are installed globally (route_task gate).");
    }
    Ok(())
}

/// Project-scoped Cursor rules (only when not using --global).
pub fn install_project_cursor_rules(quiet: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    let rules_dir = root.join(".cursor").join("rules");
    fs::create_dir_all(&rules_dir).with_context(|| format!("create {}", rules_dir.display()))?;

    let rule_path = rules_dir.join("agent-brain.mdc");
    fs::write(&rule_path, CURSOR_RULE).with_context(|| format!("write {}", rule_path.display()))?;
    let mode_path = rules_dir.join("agent-brain-mode.mdc");
    let mode_body = format!(
        "---\ndescription: agent-brain mode — short on-ramp for routed memory and skills\nalwaysApply: false\n---\n\n{}",
        host_install::AGENT_BRAIN_MODE_SNIPPET
    );
    fs::write(&mode_path, &mode_body).with_context(|| format!("write {}", mode_path.display()))?;
    if !quiet {
        println!("Installed project Cursor rule at {}", rule_path.display());
        println!("Installed project Cursor mode at {}", mode_path.display());
    }
    Ok(())
}

fn install_cursor_hooks(quiet: bool) -> Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let cursor_dir = home.join(".cursor");
    let hooks_dir = cursor_dir.join("hooks").join("agent-brain");
    fs::create_dir_all(&hooks_dir).with_context(|| format!("create {}", hooks_dir.display()))?;

    let script_path = hooks_dir.join("route_gate.py");
    fs::write(&script_path, ROUTE_GATE_HOOK)
        .with_context(|| format!("write {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;
    }

    let hooks_config = cursor_dir.join("hooks.json");
    let merged = merge_hooks_config(&hooks_config, AGENT_BRAIN_HOOKS_JSON)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(&hooks_config, format!("{pretty}\n"))
        .with_context(|| format!("write hooks config to {}", hooks_config.display()))?;

    if !command_exists("python3") {
        eprintln!("Warning: python3 not found on PATH — route_gate hook requires python3");
    }

    if !quiet {
        println!("Installed Cursor hooks at {}", hooks_config.display());
    }
    Ok(())
}

const AGENT_BRAIN_MCP_ALLOW: &str = "agent-brain:*";

fn install_cursor_permissions(quiet: bool) -> Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let cursor_dir = home.join(".cursor");
    fs::create_dir_all(&cursor_dir).with_context(|| format!("create {}", cursor_dir.display()))?;

    let path = cursor_dir.join("permissions.json");
    let merged = merge_permissions_config(&path)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(&path, format!("{pretty}\n"))
        .with_context(|| format!("write permissions config to {}", path.display()))?;

    if !quiet {
        println!("Installed Cursor MCP allowlist at {}", path.display());
        println!(
            "  Added {AGENT_BRAIN_MCP_ALLOW} — CLI agents skip per-session MCP approval when Run Mode is enabled."
        );
    }
    Ok(())
}

fn merge_permissions_config(path: &Path) -> Result<Value> {
    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !root.is_object() {
        root = json!({});
    }

    let existing = root
        .get("mcpAllowlist")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let has_agent_brain = existing.iter().any(|entry| {
        entry
            .as_str()
            .map(|s| s.eq_ignore_ascii_case(AGENT_BRAIN_MCP_ALLOW))
            .unwrap_or(false)
    });

    let mut mcp_allowlist = existing;
    if !has_agent_brain {
        mcp_allowlist.push(json!(AGENT_BRAIN_MCP_ALLOW));
    }

    root["mcpAllowlist"] = json!(mcp_allowlist);
    Ok(root)
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn merge_hooks_config(path: &Path, agent_brain_fragment: &str) -> Result<Value> {
    let fragment: Value =
        serde_json::from_str(agent_brain_fragment).context("parse agent-brain hooks")?;
    let fragment_hooks = fragment
        .get("hooks")
        .and_then(|v| v.as_object())
        .context("agent-brain hooks fragment missing hooks object")?;

    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({ "version": 1, "hooks": {} })
    };

    if root.get("version").is_none() {
        root["version"] = json!(1);
    }

    let hooks = root
        .as_object_mut()
        .and_then(|o| o.get_mut("hooks"))
        .and_then(|v| v.as_object_mut())
        .context("hooks.json must contain a hooks object")?;

    for (event, entries) in fragment_hooks {
        let existing = hooks.entry(event.clone()).or_insert_with(|| json!([]));
        let Some(arr) = existing.as_array_mut() else {
            continue;
        };
        arr.retain(|entry| !is_agent_brain_hook_entry(entry));
        if let Some(new_entries) = entries.as_array() {
            for entry in new_entries {
                arr.push(entry.clone());
            }
        }
    }

    Ok(root)
}

fn is_agent_brain_hook_entry(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(|v| v.as_str())
        .map(|cmd| cmd.contains("hooks/agent-brain/route_gate.py"))
        .unwrap_or(false)
}

const ROUTE_GATE_HOOK: &str = include_str!("../hooks/route_gate.py");

const AGENT_BRAIN_HOOKS_JSON: &str = include_str!("../hooks/agent-brain-hooks.json");

const CURSOR_RULE: &str = r#"---
description: Require agent-brain MCP on every agent turn (route_task, store_memory)
alwaysApply: true
---

# agent-brain MCP (required)

You have an MCP server named **agent-brain**. You must use it — do not improvise routing from stale context.

## Every user turn (before planning or editing)

1. Call **`route_task`** with:
   - `user_message`: the user's latest message (verbatim intent)
   - `current_working_directory`: workspace root or cwd
   - `open_files`: paths the user has open (if known)
2. Treat the response as authoritative for this turn:
   - Load **`recommended_skills`** and **`recommended_agents`** (use returned `path` values)
   - Apply **`applicable_rules`** and **`must_apply`** to your plan
   - Use **`relevant_memory`** for project conventions

Do **not** skip `route_task`. Do **not** load skills/rules from guesswork when agent-brain is available.

## Task completion

When a durable decision or convention is established, call **`store_memory`** once (max 50 words, no secrets).

## Fallback

If agent-brain MCP tools are unavailable, set `AGENT_BRAIN_ROUTE_HOOKS=0` and proceed with reduced context.

## User visibility (no need to expand MCP JSON)

Each `route_task` writes a readable summary to **`~/.agent_brain/logs/last-route.md`**. The user can run **`agent-brain briefing`** in a terminal or watch the MCP output panel for a one-line stderr summary. The JSON field **`briefing`** is a short one-liner; full detail is in the file.

## macOS

Linker-signed local builds are killed by taskgated when Cursor launches MCP. Use **`make release-macos`**, **`agent-brain doctor --fix`**, or the GitHub release binary at `~/.local/bin/agent-brain`.

**Hooks enforce this:** Cursor blocks other tools until `route_task` succeeds each turn.
"#;

pub fn mcp_config_path(global: bool) -> Result<PathBuf> {
    if global {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".cursor").join("mcp.json"));
    }
    Ok(std::env::current_dir()?.join(".cursor").join("mcp.json"))
}

pub fn mcp_server_entry(exe: &Path) -> Value {
    let build_id = format!(
        "{}@{}",
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().timestamp()
    );
    let cache_dir = crate::embed::default_cache_dir();
    json!({
        "command": exe.display().to_string(),
        "args": ["serve"],
        "env": {
            "RUST_LOG": "agent_brain=info",
            "FASTEMBED_CACHE_DIR": cache_dir.display().to_string(),
            "AGENT_BRAIN_BUILD": build_id
        }
    })
}

pub fn merge_mcp_config(path: &Path, server_entry: Value) -> Result<Value> {
    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({ "mcpServers": {} })
    };

    let servers = root
        .as_object_mut()
        .and_then(|o| o.get_mut("mcpServers"))
        .and_then(|v| v.as_object_mut())
        .context("mcp.json must contain an mcpServers object")?;

    servers.insert("agent-brain".to_string(), server_entry);
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cursor_rule_requires_route_task() {
        assert!(CURSOR_RULE.contains("route_task"));
        assert!(CURSOR_RULE.contains("alwaysApply: true"));
    }

    #[test]
    fn merges_hooks_without_clobbering_other_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hooks.json");
        fs::write(
            &path,
            r#"{
  "version": 1,
  "hooks": {
    "preToolUse": [{ "command": "./hooks/custom.sh" }]
  }
}"#,
        )
        .unwrap();

        let merged = merge_hooks_config(&path, AGENT_BRAIN_HOOKS_JSON).unwrap();
        let text = serde_json::to_string_pretty(&merged).unwrap();
        assert!(text.contains("custom.sh"));
        assert!(text.contains("route_gate.py"));
        assert!(text.contains("beforeSubmitPrompt"));
        assert!(text.contains("postToolUse"));
    }

    #[test]
    fn merges_permissions_without_clobbering() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("permissions.json");
        fs::write(
            &path,
            r#"{
  "mcpAllowlist": ["github:*"],
  "terminalAllowlist": ["git"]
}"#,
        )
        .unwrap();

        let merged = merge_permissions_config(&path).unwrap();
        let text = serde_json::to_string_pretty(&merged).unwrap();
        assert!(text.contains("github:*"));
        assert!(text.contains("agent-brain:*"));
        assert!(text.contains("terminalAllowlist"));
    }

    #[test]
    fn detects_homebrew_managed_paths() {
        assert!(is_homebrew_managed(Path::new(
            "/opt/homebrew/Cellar/autonomic-stack/0.5.12/bin/agent-brain"
        )));
        assert!(!is_homebrew_managed(Path::new("/Users/me/.local/bin/agent-brain")));
    }

    #[test]
    fn merges_into_existing_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(
            &path,
            r#"{
  "mcpServers": {
    "other": { "command": "other-mcp" }
  }
}"#,
        )
        .unwrap();
        let merged = merge_mcp_config(
            &path,
            mcp_server_entry(Path::new("/usr/local/bin/agent-brain")),
        )
        .unwrap();
        let text = serde_json::to_string_pretty(&merged).unwrap();
        assert!(text.contains("agent-brain"));
        assert!(text.contains("other-mcp"));
    }
}
