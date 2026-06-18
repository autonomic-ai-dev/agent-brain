//! Install route_task gate hooks for non-Cursor hosts (Claude Code, Gemini, OpenCode, …).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

const ROUTE_GATE_PY: &str = include_str!("../hooks/route_gate.py");
const OPENCODE_ROUTE_GATE_TS: &str = include_str!("../hooks/opencode/route-gate.ts");

/// Copy `route_gate.py` into `dest_dir/route_gate.py` and return the path.
pub fn deploy_route_gate_script(dest_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dest_dir).with_context(|| format!("create {}", dest_dir.display()))?;
    let script = dest_dir.join("route_gate.py");
    fs::write(&script, ROUTE_GATE_PY).with_context(|| format!("write {}", script.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms)?;
    }
    Ok(script)
}

pub fn deploy_opencode_plugin(dest_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dest_dir).with_context(|| format!("create {}", dest_dir.display()))?;
    let plugin = dest_dir.join("agent-brain-route-gate.ts");
    fs::write(&plugin, OPENCODE_ROUTE_GATE_TS)
        .with_context(|| format!("write {}", plugin.display()))?;
    Ok(plugin)
}

fn python3_command(script: &Path) -> String {
    format!("python3 {}", script.display())
}

fn claude_code_hooks_fragment(script: &Path) -> Value {
    let cmd = python3_command(script);
    json!({
        "hooks": {
            "UserPromptSubmit": [{
                "hooks": [{
                    "type": "command",
                    "command": cmd,
                    "timeout": 30
                }]
            }],
            "PreToolUse": [{
                "matcher": ".*",
                "hooks": [{
                    "type": "command",
                    "command": cmd,
                    "timeout": 30
                }]
            }],
            "PostToolUse": [{
                "matcher": ".*",
                "hooks": [{
                    "type": "command",
                    "command": cmd,
                    "timeout": 30
                }]
            }]
        }
    })
}

fn gemini_hooks_fragment(script: &Path) -> Value {
    let cmd = python3_command(script);
    json!({
        "hooks": {
            "BeforeAgent": [{
                "hooks": [{
                    "type": "command",
                    "name": "agent-brain-route-gate",
                    "command": cmd,
                    "timeout": 30000
                }]
            }],
            "BeforeTool": [{
                "matcher": ".*",
                "hooks": [{
                    "type": "command",
                    "name": "agent-brain-route-gate",
                    "command": cmd,
                    "timeout": 30000
                }]
            }],
            "AfterTool": [{
                "matcher": ".*",
                "hooks": [{
                    "type": "command",
                    "name": "agent-brain-route-gate",
                    "command": cmd,
                    "timeout": 30000
                }]
            }]
        }
    })
}

pub fn claude_code_settings_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".claude").join("settings.json"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".claude").join("settings.json"))
}

pub fn claude_code_hooks_dir(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".claude").join("hooks").join("agent-brain"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".claude").join("hooks").join("agent-brain"))
}

pub fn gemini_hooks_dir(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".gemini").join("hooks").join("agent-brain"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".gemini").join("hooks").join("agent-brain"))
}

pub fn opencode_hooks_dir(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".config").join("opencode").join("hooks").join("agent-brain"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".opencode").join("hooks").join("agent-brain"))
}

pub fn opencode_plugin_dir(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".config").join("opencode").join("plugin"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".opencode").join("plugin"))
}

pub fn codex_hooks_dir(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".codex").join("hooks").join("agent-brain"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".codex").join("hooks").join("agent-brain"))
}

pub fn codex_hooks_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".codex").join("hooks.json"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".codex").join("hooks.json"))
}

fn codex_hooks_fragment(script: &Path) -> Value {
    claude_code_hooks_fragment(script)
}

pub fn merge_settings_hooks(path: &Path, hooks_fragment: &Value) -> Result<Value> {
    let fragment_hooks = hooks_fragment
        .get("hooks")
        .and_then(|v| v.as_object())
        .context("hooks fragment missing hooks object")?;

    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({})
    };

    if !root.is_object() {
        anyhow::bail!("{} must be a JSON object", path.display());
    }

    let hooks = root
        .as_object_mut()
        .and_then(|o| o.get_mut("hooks"))
        .and_then(|v| v.as_object_mut());

    match hooks {
        Some(hooks) => merge_hook_events(hooks, fragment_hooks, "route_gate.py"),
        None => {
            root["hooks"] = hooks_fragment.get("hooks").cloned().unwrap_or(json!({}));
        }
    }

    Ok(root)
}

fn merge_hook_events(
    hooks: &mut serde_json::Map<String, Value>,
    fragment_hooks: &serde_json::Map<String, Value>,
    marker: &str,
) {
    for (event, entries) in fragment_hooks {
        let existing = hooks
            .entry(event.clone())
            .or_insert_with(|| json!([]));
        let Some(arr) = existing.as_array_mut() else {
            continue;
        };
        arr.retain(|entry| !is_agent_brain_hook_entry(entry, marker));
        if let Some(new_entries) = entries.as_array() {
            for entry in new_entries {
                arr.push(entry.clone());
            }
        }
    }
}

fn is_agent_brain_hook_entry(entry: &Value, marker: &str) -> bool {
    if let Some(cmd) = entry.get("command").and_then(|v| v.as_str()) {
        if cmd.contains(marker) || cmd.contains("agent-brain-route-gate") {
            return true;
        }
    }
    if let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) {
        return hooks.iter().any(|h| is_agent_brain_hook_entry(h, marker));
    }
    false
}

pub fn write_settings_hooks(path: &Path, hooks_fragment: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let merged = merge_settings_hooks(path, hooks_fragment)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(path, format!("{pretty}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn install_claude_code_hooks(user: bool, quiet: bool) -> Result<PathBuf> {
    let hooks_dir = claude_code_hooks_dir(user)?;
    let script = deploy_route_gate_script(&hooks_dir)?;
    let settings = claude_code_settings_path(user)?;
    write_settings_hooks(&settings, &claude_code_hooks_fragment(&script))?;
    if !quiet {
        println!("Installed Claude Code route gate hooks at {}", settings.display());
    }
    Ok(settings)
}

pub fn install_gemini_hooks(user: bool, settings_path: &Path, quiet: bool) -> Result<()> {
    let hooks_dir = gemini_hooks_dir(user)?;
    let script = deploy_route_gate_script(&hooks_dir)?;
    write_settings_hooks(settings_path, &gemini_hooks_fragment(&script))?;
    if !quiet {
        println!("Installed Gemini CLI route gate hooks in {}", settings_path.display());
    }
    Ok(())
}

pub fn install_opencode_hooks(user: bool, quiet: bool) -> Result<()> {
    let hooks_dir = opencode_hooks_dir(user)?;
    let _script = deploy_route_gate_script(&hooks_dir)?;
    let plugin_dir = opencode_plugin_dir(user)?;
    let plugin = deploy_opencode_plugin(&plugin_dir)?;
    if !quiet {
        println!("Installed OpenCode route gate plugin at {}", plugin.display());
        println!("  OpenCode loads plugins from ~/.config/opencode/plugin/ (or .opencode/plugin/).");
    }
    Ok(())
}

pub fn install_codex_hooks(user: bool, quiet: bool) -> Result<()> {
    let hooks_dir = codex_hooks_dir(user)?;
    let script = deploy_route_gate_script(&hooks_dir)?;
    let hooks_path = codex_hooks_path(user)?;
    write_settings_hooks(&hooks_path, &codex_hooks_fragment(&script))?;
    if !quiet {
        println!("Installed Codex route gate hooks at {}", hooks_path.display());
        println!("  Review and trust hooks in Codex with `/hooks` if prompted.");
    }
    Ok(())
}

pub fn install_vscode_copilot_instructions(quiet: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    let path = root.join(".github").join("copilot-instructions.md");
    if path.is_file() {
        if let Ok(existing) = fs::read_to_string(&path) {
            if existing.contains("instructions-version: 5") {
                return Ok(());
            }
        }
    }
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, VSCODE_COPILOT_INSTRUCTIONS)
        .with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!("Installed GitHub Copilot instructions at {}", path.display());
    }
    Ok(())
}

const VSCODE_COPILOT_INSTRUCTIONS: &str = r"# agent-brain MCP (GitHub Copilot / VS Code)
instructions-version: 5

VS Code and GitHub Copilot do not expose Cursor-style PreToolUse hooks. Enforcement is:

1. Connect agent-brain MCP (`agent-brain install --vscode [--global]`).
2. Call **`route_task`** at the start of every user turn before planning or edits.
3. Use agent-brain token tools (`grep_search`, `file_summary`, `read_file_head/tail`) instead of unbounded workspace search.
4. Call **`store_memory`** at task end for durable outcomes (max 50 words, no secrets).

Session digests from Cursor, OpenCode, Codex, Gemini, and Antigravity only surface through `route_task`.
";

pub fn hooks_status(path: &Path, marker: &str) -> &'static str {
    if !path.is_file() {
        return "not configured";
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return "unreadable";
    };
    if raw.contains(marker) || raw.contains("agent-brain-route-gate") {
        "OK"
    } else {
        "missing hooks"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn merges_gemini_hooks_into_settings() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
        let script = dir.path().join("route_gate.py");
        fs::write(&script, "#!/usr/bin/env python3\n").unwrap();
        let fragment = gemini_hooks_fragment(&script);
        let merged = merge_settings_hooks(&path, &fragment).unwrap();
        assert_eq!(merged["theme"], "dark");
        assert!(merged["hooks"]["BeforeTool"].is_array());
    }

    #[test]
    fn claude_settings_path_project_scope() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let path = claude_code_settings_path(false).unwrap();
        assert!(path.ends_with(".claude/settings.json"));
    }
}
