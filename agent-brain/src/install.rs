use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(global: bool, print_only: bool, reload: bool) -> Result<()> {
    let exe = std::env::current_exe().context("resolve agent-brain binary path")?;
    let config_path = mcp_config_path(global)?;
    let snippet = mcp_server_entry(&exe);

    if print_only {
        println!("{}", serde_json::to_string_pretty(&json!({ "mcpServers": { "agent-brain": snippet } }))?);
        return Ok(());
    }

    configure_cursor(global, &exe, false)?;
    println!("agent-brain MCP configured at {}", config_path.display());
    println!("Binary: {}", exe.display());
    println!();
    if reload {
        println!("Reload nudge: refreshed mcp.json (AGENT_BRAIN_BUILD bumped).");
        println!("Cursor may reload agent-brain automatically; if not, toggle it under Settings → MCP.");
    } else {
        println!("Next steps:");
        println!("  1. Restart Cursor or toggle agent-brain under Settings → MCP (loads new binary + hooks)");
        println!("  2. Confirm 'agent-brain' appears and is enabled under MCP");
        println!("  3. Confirm hooks loaded under Settings → Hooks (route_task gate)");
        println!("  4. Confirm ~/.cursor/permissions.json includes agent-brain:* (CLI MCP auto-approve)");
        println!("  5. Confirm project rule at .cursor/rules/agent-brain.mdc (Settings → Rules)");
        println!("  6. Use Agent mode — route_task is required before other agent-brain MCP tools");
        println!();
        println!("After local rebuilds without full reinstall: agent-brain install --global --reload");
    }
    Ok(())
}

/// Merge MCP config and optionally refresh Cursor hooks/rule (used by install + auto-update).
pub fn configure_cursor(global: bool, exe: &Path, quiet: bool) -> Result<()> {
    let config_path = mcp_config_path(global)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let merged = merge_mcp_config(&config_path, mcp_server_entry(exe))?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(&config_path, format!("{pretty}\n")).with_context(|| {
        format!("write MCP config to {}", config_path.display())
    })?;

    if global {
        install_cursor_hooks(quiet)?;
        install_cursor_permissions(quiet)?;
    }
    install_project_cursor_rules(quiet)?;

    #[cfg(target_os = "macos")]
    {
        if let Err(err) = crate::doctor::adhoc_sign(exe) {
            if !quiet {
                eprintln!("Warning: adhoc codesign failed: {err}");
            }
        }
    }

    Ok(())
}

/// Cursor loads **project** rules from `<workspace>/.cursor/rules/*.mdc` only.
/// `~/.cursor/rules/` is not read by Cursor — use Settings → Rules for User Rules.
pub fn install_project_cursor_rules(quiet: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    let rules_dir = root.join(".cursor").join("rules");
    fs::create_dir_all(&rules_dir).with_context(|| format!("create {}", rules_dir.display()))?;

    let rule_path = rules_dir.join("agent-brain.mdc");
    fs::write(&rule_path, CURSOR_RULE).with_context(|| format!("write {}", rule_path.display()))?;
    if !quiet {
        println!("Installed project Cursor rule at {}", rule_path.display());
        println!(
            "Note: Cursor does not load ~/.cursor/rules/. For global text rules, use Cursor Settings → Rules → User Rules."
        );
    }
    Ok(())
}

fn install_cursor_hooks(quiet: bool) -> Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let cursor_dir = home.join(".cursor");
    let hooks_dir = cursor_dir.join("hooks").join("agent-brain");
    fs::create_dir_all(&hooks_dir).with_context(|| format!("create {}", hooks_dir.display()))?;

    let script_path = hooks_dir.join("route_gate.py");
    fs::write(&script_path, ROUTE_GATE_HOOK).with_context(|| format!("write {}", script_path.display()))?;
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
    fs::write(&hooks_config, format!("{pretty}\n")).with_context(|| {
        format!("write hooks config to {}", hooks_config.display())
    })?;

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
    fs::write(&path, format!("{pretty}\n")).with_context(|| {
        format!("write permissions config to {}", path.display())
    })?;

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
    let fragment: Value = serde_json::from_str(agent_brain_fragment).context("parse agent-brain hooks")?;
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
        let existing = hooks
            .entry(event.clone())
            .or_insert_with(|| json!([]));
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

fn mcp_config_path(global: bool) -> Result<PathBuf> {
    if global {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".cursor").join("mcp.json"));
    }
    Ok(std::env::current_dir()?.join(".cursor").join("mcp.json"))
}

fn mcp_server_entry(exe: &Path) -> Value {
    let build_id = format!(
        "{}@{}",
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().timestamp()
    );
    json!({
        "command": exe.display().to_string(),
        "args": ["serve"],
        "env": {
            "RUST_LOG": "agent_brain=info",
            "AGENT_BRAIN_BOOTSTRAP_BG": "1",
            "AGENT_BRAIN_BOOTSTRAP_DELAY_SEC": "2",
            "AGENT_BRAIN_BOOTSTRAP_INTERVAL_SEC": "3600",
            "AGENT_BRAIN_AUTO_UPDATE_DELAY_SEC": "60",
            "AGENT_BRAIN_SESSION_INGEST_DELAY_SEC": "180",
            "AGENT_BRAIN_BUILD": build_id
        }
    })
}

fn merge_mcp_config(path: &Path, server_entry: Value) -> Result<Value> {
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
        let merged = merge_mcp_config(&path, mcp_server_entry(Path::new("/usr/local/bin/agent-brain"))).unwrap();
        let text = serde_json::to_string_pretty(&merged).unwrap();
        assert!(text.contains("agent-brain"));
        assert!(text.contains("other-mcp"));
    }
}
