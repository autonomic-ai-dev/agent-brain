use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn claude_code_settings_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".claude").join("settings.json"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".claude").join("settings.json"))
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
        Some(hooks) => merge_hook_events(hooks, fragment_hooks),
        None => {
            root["hooks"] = hooks_fragment.get("hooks").cloned().unwrap_or(json!({}));
        }
    }

    Ok(root)
}

fn merge_hook_events(
    hooks: &mut serde_json::Map<String, Value>,
    fragment_hooks: &serde_json::Map<String, Value>,
) {
    for (event, entries) in fragment_hooks {
        let existing = hooks.entry(event.clone()).or_insert_with(|| json!([]));
        let Some(arr) = existing.as_array_mut() else {
            continue;
        };
        arr.retain(|entry| !is_tekmor_hook_entry(entry));
        if let Some(new_entries) = entries.as_array() {
            for entry in new_entries {
                arr.push(entry.clone());
            }
        }
    }
}

fn is_tekmor_hook_entry(entry: &Value) -> bool {
    if let Some(cmd) = entry.get("command").and_then(|v| v.as_str()) {
        if is_gate_subcommand(cmd) {
            return true;
        }
    }
    if let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) {
        return hooks.iter().any(is_tekmor_hook_entry);
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

fn tekmor_subcommand(subcommand: &str) -> Result<String> {
    let exe = std::env::current_exe().context("resolve tekmor binary")?;
    let quoted = exe.display().to_string().replace('\'', r"'\''");
    Ok(format!("'{quoted}' {subcommand}"))
}

fn is_gate_subcommand(cmd: &str) -> bool {
    let cmd = cmd.trim_end();
    cmd.ends_with(" stop-gate") || cmd.ends_with(" commit-gate") || cmd.ends_with(" bash-gate")
}

pub fn install_claude_code_hooks(user: bool, quiet: bool) -> Result<PathBuf> {
    let settings = claude_code_settings_path(user)?;
    let fragment = json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash",
                "hooks": [{
                    "type": "command",
                    "command": tekmor_subcommand("bash-gate")?,
                    "timeout": 30
                }]
            }],
            "Stop": [{
                "hooks": [{
                    "type": "command",
                    "command": tekmor_subcommand("stop-gate")?,
                    "timeout": crate::verify::MAX_STEP_TIMEOUT_SECS + 60
                }]
            }]
        }
    });
    write_settings_hooks(&settings, &fragment)?;
    if !quiet {
        println!("Installed Claude Code gate hooks at {}", settings.display());
    }
    Ok(settings)
}
