use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::install::{mcp_server_entry, merge_mcp_config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostTarget {
    Cursor { global: bool },
    ClaudeDesktop,
    VsCode { user: bool },
    ClaudeCode { user: bool },
    OpenCode { user: bool },
    All,
}

impl HostTarget {
    pub fn from_args(args: &[String]) -> Self {
        if args.iter().any(|a| a == "--all") {
            return Self::All;
        }
        let global = args.iter().any(|a| a == "--global");
        if args.iter().any(|a| a == "--claude-desktop") {
            return Self::ClaudeDesktop;
        }
        if args.iter().any(|a| a == "--vscode") {
            return Self::VsCode { user: global };
        }
        if args.iter().any(|a| a == "--claude-code") {
            return Self::ClaudeCode { user: global };
        }
        if args.iter().any(|a| a == "--opencode") {
            return Self::OpenCode { user: global };
        }
        Self::Cursor { global }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cursor { global } if global => "cursor (global)",
            Self::Cursor { .. } => "cursor (workspace)",
            Self::ClaudeDesktop => "claude-desktop",
            Self::VsCode { user: true } => "vscode (user)",
            Self::VsCode { .. } => "vscode (workspace)",
            Self::ClaudeCode { user: true } => "claude-code (user)",
            Self::ClaudeCode { .. } => "claude-code (project)",
            Self::OpenCode { user: true } => "opencode (user)",
            Self::OpenCode { .. } => "opencode (project)",
            Self::All => "all hosts",
        }
    }
}

pub fn install_host(target: HostTarget, exe: &Path, quiet: bool) -> Result<Vec<PathBuf>> {
    match target {
        HostTarget::All => {
            let mut paths = Vec::new();
            paths.extend(install_host(HostTarget::Cursor { global: true }, exe, true)?);
            paths.extend(install_host(HostTarget::ClaudeDesktop, exe, true)?);
            paths.extend(install_host(HostTarget::VsCode { user: true }, exe, true)?);
            paths.extend(install_host(HostTarget::ClaudeCode { user: true }, exe, true)?);
            paths.extend(install_host(HostTarget::OpenCode { user: true }, exe, true)?);
            if !quiet {
                println!("Installed agent-brain MCP for: cursor, claude-desktop, vscode, claude-code, opencode");
            }
            Ok(paths)
        }
        HostTarget::Cursor { global } => crate::install::configure_cursor(global, exe, quiet).map(|_| {
            vec![crate::install::mcp_config_path(global).expect("cursor mcp path")]
        }),
        HostTarget::ClaudeDesktop => install_claude_desktop(exe, quiet),
        HostTarget::VsCode { user } => install_vscode(exe, user, quiet),
        HostTarget::ClaudeCode { user } => install_claude_code(exe, user, quiet),
        HostTarget::OpenCode { user } => install_opencode(exe, user, quiet),
    }
}

fn install_claude_desktop(exe: &Path, quiet: bool) -> Result<Vec<PathBuf>> {
    let path = claude_desktop_config_path()?;
    write_mcp_servers_file(&path, mcp_server_entry(exe))?;
    if !quiet {
        println!("Installed Claude Desktop MCP at {}", path.display());
        println!("  Restart Claude Desktop fully (Cmd+Q) to load agent-brain.");
    }
    Ok(vec![path])
}

fn install_vscode(exe: &Path, user: bool, quiet: bool) -> Result<Vec<PathBuf>> {
    let path = vscode_mcp_path(user)?;
    write_vscode_servers_file(&path, vscode_server_entry(exe))?;
    if !quiet {
        println!("Installed VS Code MCP at {}", path.display());
        println!("  Run \"MCP: List Servers\" or reload the window to connect.");
    }
    Ok(vec![path])
}

pub fn claude_desktop_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home directory")?;
    #[cfg(target_os = "macos")]
    {
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("Claude")
            .join("claude_desktop_config.json"));
    }
    #[cfg(target_os = "windows")]
    {
        return Ok(std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("Claude")
            .join("claude_desktop_config.json"));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(home.join(".config").join("Claude").join("claude_desktop_config.json"))
    }
}

pub fn vscode_mcp_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        #[cfg(target_os = "macos")]
        {
            return Ok(home
                .join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
                .join("mcp.json"));
        }
        #[cfg(target_os = "windows")]
        {
            return Ok(std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Roaming"))
                .join("Code")
                .join("User")
                .join("mcp.json"));
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            return Ok(home.join(".config").join("Code").join("User").join("mcp.json"));
        }
    }

    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".vscode").join("mcp.json"))
}

pub fn claude_code_mcp_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".claude.json"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".mcp.json"))
}

pub fn opencode_config_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        #[cfg(target_os = "macos")]
        {
            return Ok(home.join(".config").join("opencode").join("opencode.json"));
        }
        #[cfg(target_os = "windows")]
        {
            return Ok(std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Roaming"))
                .join("opencode")
                .join("opencode.json"));
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            return Ok(home.join(".config").join("opencode").join("opencode.json"));
        }
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join("opencode.json"))
}

fn install_opencode(exe: &Path, user: bool, quiet: bool) -> Result<Vec<PathBuf>> {
    let path = opencode_config_path(user)?;
    write_opencode_config(&path, opencode_server_entry(exe))?;
    install_opencode_instructions(user, quiet)?;
    if !quiet {
        println!("Installed OpenCode MCP at {}", path.display());
        if user {
            println!("  User scope: ~/.config/opencode/opencode.json");
        } else {
            println!("  Project scope: opencode.json at repository root.");
        }
        println!("  Restart OpenCode or run `opencode mcp list` to verify.");
    }
    Ok(vec![path])
}

pub fn opencode_server_entry_public(exe: &Path) -> Value {
    opencode_server_entry(exe)
}

fn opencode_server_entry(exe: &Path) -> Value {
    let base = mcp_server_entry(exe);
    let cmd = base
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| exe.display().to_string());
    let mut command = vec![json!(cmd)];
    if let Some(args) = base.get("args").and_then(|v| v.as_array()) {
        command.extend(args.iter().cloned());
    }
    let mut entry = json!({
        "type": "local",
        "command": command,
        "enabled": true,
    });
    if let Some(env) = base.get("env") {
        entry["environment"] = env.clone();
    }
    entry
}

pub fn merge_opencode_config(path: &Path, server_entry: Value) -> Result<Value> {
    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({
            "$schema": "https://opencode.ai/config.json",
            "mcp": {}
        })
    };

    if !root.is_object() {
        bail!("{} must be a JSON object", path.display());
    }

    if root.get("$schema").is_none() {
        root["$schema"] = json!("https://opencode.ai/config.json");
    }

    let mcp = root
        .as_object_mut()
        .and_then(|o| o.get_mut("mcp"))
        .and_then(|v| v.as_object_mut());

    match mcp {
        Some(mcp) => {
            mcp.insert("agent-brain".to_string(), server_entry);
        }
        None => {
            root["mcp"] = json!({ "agent-brain": server_entry });
        }
    }
    Ok(root)
}

fn write_opencode_config(path: &Path, server_entry: Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let merged = merge_opencode_config(path, server_entry)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(path, format!("{pretty}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn install_opencode_instructions(user: bool, quiet: bool) -> Result<()> {
    let path = if user {
        let home = dirs::home_dir().context("home directory")?;
        let dir = home.join(".config").join("opencode");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    } else {
        let cwd = std::env::current_dir().context("current working directory")?;
        let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
        let dir = root.join(".opencode");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    };
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, OPENCODE_INSTRUCTIONS).with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!("Installed OpenCode instructions at {}", path.display());
    }
    Ok(())
}

const OPENCODE_INSTRUCTIONS: &str = r#"# agent-brain MCP (required)

At the start of every user turn, call the **agent-brain** MCP tool **`route_task`**
with `user_message`, `current_working_directory`, and `open_files` when known.

- Load skills/agents from `recommended_skills` / `recommended_agents` paths.
- Apply `applicable_rules` and `must_apply`.
- At task end, call **`store_memory`** for durable outcomes (max 50 words, no secrets).

Readable summary: `~/.agent_brain/logs/last-route.md` or `agent-brain briefing`.
"#;

fn write_mcp_servers_file(path: &Path, server_entry: Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let merged = merge_mcp_config(path, server_entry)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(path, format!("{pretty}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn write_vscode_servers_file(path: &Path, server_entry: Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let merged = merge_vscode_mcp_config(path, server_entry)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(path, format!("{pretty}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn merge_vscode_mcp_config(path: &Path, server_entry: Value) -> Result<Value> {
    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({ "servers": {} })
    };

    let servers = root
        .as_object_mut()
        .and_then(|o| o.get_mut("servers"))
        .and_then(|v| v.as_object_mut())
        .context("vscode mcp.json must contain a servers object")?;

    servers.insert("agent-brain".to_string(), server_entry);
    Ok(root)
}

pub fn vscode_server_entry_public(exe: &Path) -> Value {
    vscode_server_entry(exe)
}

fn vscode_server_entry(exe: &Path) -> Value {
    let base = mcp_server_entry(exe);
    let mut entry = json!({
        "type": "stdio",
        "command": base.get("command").cloned().unwrap_or_else(|| json!(exe.display().to_string())),
        "args": base.get("args").cloned().unwrap_or_else(|| json!(["serve"])),
    });
    if let Some(env) = base.get("env") {
        entry["env"] = env.clone();
    }
    entry
}

fn claude_code_server_entry(exe: &Path) -> Value {
    let base = mcp_server_entry(exe);
    let mut entry = json!({
        "type": "stdio",
        "command": base.get("command").cloned().unwrap_or_else(|| json!(exe.display().to_string())),
        "args": base.get("args").cloned().unwrap_or_else(|| json!(["serve"])),
    });
    if let Some(env) = base.get("env") {
        entry["env"] = env.clone();
    }
    entry
}

fn install_claude_code_rule(user: bool, quiet: bool) -> Result<()> {
    let path = if user {
        let home = dirs::home_dir().context("home directory")?;
        let rules_dir = home.join(".claude");
        fs::create_dir_all(&rules_dir)?;
        rules_dir.join("agent-brain.md")
    } else {
        let cwd = std::env::current_dir().context("current working directory")?;
        let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
        let rules_dir = root.join(".claude");
        fs::create_dir_all(&rules_dir)?;
        rules_dir.join("agent-brain.md")
    };
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, CLAUDE_CODE_RULE).with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!("Installed Claude Code rule template at {}", path.display());
    }
    Ok(())
}

const CLAUDE_CODE_RULE: &str = r#"# agent-brain MCP (required)

Call **`route_task`** at the start of every user turn before planning or edits.

- Pass `user_message`, `current_working_directory`, and `open_files` when known.
- Load skills/agents from `recommended_skills` / `recommended_agents` paths.
- Apply `applicable_rules` and `must_apply`.
- At task end, call **`store_memory`** for durable outcomes (max 50 words, no secrets).

If MCP is unavailable, proceed without the gate — do not guess routing from stale context.

Readable summary: `~/.agent_brain/logs/last-route.md` or `agent-brain briefing`.
"#;

pub fn merge_claude_json_mcp(path: &Path, server_entry: Value) -> Result<Value> {
    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({})
    };

    if !root.is_object() {
        bail!("{} must be a JSON object", path.display());
    }

    let servers = root
        .as_object_mut()
        .and_then(|o| o.get_mut("mcpServers"))
        .and_then(|v| v.as_object_mut());

    match servers {
        Some(servers) => {
            servers.insert("agent-brain".to_string(), server_entry);
        }
        None => {
            root["mcpServers"] = json!({ "agent-brain": server_entry });
        }
    }
    Ok(root)
}

pub fn write_claude_json_mcp(path: &Path, server_entry: Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
    }
    let merged = merge_claude_json_mcp(path, server_entry)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(path, format!("{pretty}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn install_claude_code(exe: &Path, user: bool, quiet: bool) -> Result<Vec<PathBuf>> {
    let path = claude_code_mcp_path(user)?;
    if user {
        write_claude_json_mcp(&path, claude_code_server_entry(exe))?;
    } else {
        write_mcp_servers_file(&path, claude_code_server_entry(exe))?;
    }
    install_claude_code_rule(user, quiet)?;
    if !quiet {
        println!("Installed Claude Code MCP at {}", path.display());
        if user {
            println!("  User scope: ~/.claude.json (not settings.json — that file ignores mcpServers).");
        } else {
            println!("  Project scope: .mcp.json at repository root.");
        }
        println!("  Start a new Claude Code session; run /mcp to verify.");
    }
    Ok(vec![path])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn merges_vscode_servers() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.json");
        fs::write(
            &path,
            r#"{"servers":{"other":{"type":"stdio","command":"other"}}}"#,
        )
        .unwrap();
        let merged = merge_vscode_mcp_config(
            &path,
            json!({"type":"stdio","command":"/bin/agent-brain","args":["serve"]}),
        )
        .unwrap();
        assert!(merged["servers"]["agent-brain"].is_object());
        assert!(merged["servers"]["other"].is_object());
    }

    #[test]
    fn merges_claude_json_user_scope() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
        let merged = merge_claude_json_mcp(
            &path,
            json!({"type":"stdio","command":"/bin/agent-brain","args":["serve"]}),
        )
        .unwrap();
        assert!(merged["mcpServers"]["agent-brain"].is_object());
        assert_eq!(merged["theme"], "dark");
    }

    #[test]
    fn merges_opencode_config_preserves_schema() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("opencode.json");
        fs::write(
            &path,
            r#"{"$schema":"https://opencode.ai/config.json","model":"test/model"}"#,
        )
        .unwrap();
        let merged = merge_opencode_config(
            &path,
            json!({
                "type": "local",
                "command": ["/bin/agent-brain", "serve"],
                "enabled": true
            }),
        )
        .unwrap();
        assert_eq!(merged["model"], "test/model");
        assert_eq!(merged["mcp"]["agent-brain"]["type"], "local");
    }

    #[test]
    fn host_target_parses_opencode_flag() {
        let args = vec!["install".into(), "--opencode".into(), "--global".into()];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::OpenCode { user: true }
        );
    }

    #[test]
    fn host_target_parses_flags() {
        let args = vec![
            "install".into(),
            "--claude-code".into(),
            "--global".into(),
        ];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::ClaudeCode { user: true }
        );
    }
}
