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
    Codex { user: bool },
    Gemini { user: bool },
    Antigravity { user: bool },
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
        if args.iter().any(|a| a == "--codex") {
            return Self::Codex { user: global };
        }
        if args.iter().any(|a| a == "--gemini") {
            return Self::Gemini { user: global };
        }
        if args.iter().any(|a| a == "--antigravity") {
            return Self::Antigravity { user: global };
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
            Self::Codex { user: true } => "codex (user)",
            Self::Codex { .. } => "codex (project)",
            Self::Gemini { user: true } => "gemini (user)",
            Self::Gemini { .. } => "gemini (project)",
            Self::Antigravity { user: true } => "antigravity (user)",
            Self::Antigravity { .. } => "antigravity (project)",
            Self::All => "all hosts",
        }
    }
}

pub fn install_host(target: HostTarget, exe: &Path, quiet: bool) -> Result<Vec<PathBuf>> {
    match target {
        HostTarget::All => {
            let mut paths = Vec::new();
            let mut ok_hosts = Vec::new();
            let mut errors = Vec::new();

            let hosts: &[(HostTarget, &str)] = &[
                (HostTarget::Cursor { global: true }, "cursor"),
                (HostTarget::ClaudeDesktop, "claude-desktop"),
                (HostTarget::VsCode { user: true }, "vscode"),
                (HostTarget::ClaudeCode { user: true }, "claude-code"),
                (HostTarget::OpenCode { user: true }, "opencode"),
                (HostTarget::Codex { user: true }, "codex"),
                (HostTarget::Gemini { user: true }, "gemini"),
                (HostTarget::Antigravity { user: true }, "antigravity"),
            ];

            for (target, name) in hosts {
                match install_host(target.clone(), exe, true) {
                    Ok(p) => {
                        paths.extend(p);
                        ok_hosts.push(*name);
                    }
                    Err(e) => errors.push(format!("  ✗ {name}: {e}")),
                }
            }

            if !quiet {
                if !ok_hosts.is_empty() {
                    println!("Installed agent-brain MCP for: {}", ok_hosts.join(", "));
                }
                for err in &errors {
                    eprintln!("{err}");
                }
                if !errors.is_empty() {
                    eprintln!("Note: some hosts failed — typically because the integration is not installed. Use `agent-brain install <host>` for individual retries.");
                }
            }

            Ok(paths)
        }
        HostTarget::Cursor { global } => crate::install::configure_cursor(global, exe, quiet)
            .map(|_| vec![crate::install::mcp_config_path(global).expect("cursor mcp path")]),
        HostTarget::ClaudeDesktop => install_claude_desktop(exe, quiet),
        HostTarget::VsCode { user } => install_vscode(exe, user, quiet),
        HostTarget::ClaudeCode { user } => install_claude_code(exe, user, quiet),
        HostTarget::OpenCode { user } => install_opencode(exe, user, quiet),
        HostTarget::Codex { user } => install_codex(exe, user, quiet),
        HostTarget::Gemini { user } => install_gemini(exe, user, quiet),
        HostTarget::Antigravity { user } => install_antigravity(exe, user, quiet),
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
    let _ = crate::host_hooks::install_vscode_copilot_instructions(quiet);
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
        Ok(home
            .join(".config")
            .join("Claude")
            .join("claude_desktop_config.json"))
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
            return Ok(home
                .join(".config")
                .join("Code")
                .join("User")
                .join("mcp.json"));
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
    write_opencode_config(&path, opencode_server_entry(exe), user)?;
    install_opencode_instructions(user, quiet)?;
    crate::host_hooks::install_opencode_hooks(user, quiet)?;
    if !quiet {
        println!("Installed OpenCode MCP at {}", path.display());
        if user {
            println!("  User scope: ~/.config/opencode/opencode.json");
        } else {
            println!("  Project scope: opencode.json at repository root.");
        }
        println!("  Restart OpenCode or run `opencode mcp list` to verify.");
        println!("  Route gate plugin: agent-brain-route-gate.ts in opencode/plugin/.");
    }
    Ok(vec![path])
}

pub fn gemini_config_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".gemini").join("settings.json"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".gemini").join("settings.json"))
}

pub fn antigravity_config_paths(user: bool) -> Result<Vec<PathBuf>> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(vec![
            home.join(".gemini")
                .join("antigravity")
                .join("mcp_config.json"),
            home.join(".gemini").join("config").join("mcp_config.json"),
        ]);
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(vec![root
        .join(".gemini")
        .join("antigravity")
        .join("mcp_config.json")])
}

fn install_gemini(exe: &Path, user: bool, quiet: bool) -> Result<Vec<PathBuf>> {
    let path = gemini_config_path(user)?;
    write_claude_json_mcp(&path, mcp_server_entry(exe))?;
    install_gemini_instructions(user, quiet)?;
    crate::host_hooks::install_gemini_hooks(user, &path, quiet)?;
    if !quiet {
        println!("Installed Gemini CLI MCP at {}", path.display());
        if user {
            println!("  User scope: ~/.gemini/settings.json");
        } else {
            println!("  Project scope: .gemini/settings.json at repository root.");
        }
        println!("  Restart Gemini CLI or run `gemini mcp list` to verify.");
        println!("  Route gate hooks: BeforeAgent / BeforeTool in settings.json (`/hooks panel`).");
    }
    Ok(vec![path])
}

fn install_antigravity(exe: &Path, user: bool, quiet: bool) -> Result<Vec<PathBuf>> {
    let paths = antigravity_config_paths(user)?;
    let entry = mcp_server_entry(exe);
    let mut written = Vec::new();
    for path in &paths {
        write_mcp_servers_file(path, entry.clone())?;
        written.push(path.clone());
    }
    install_antigravity_instructions(user, quiet)?;
    if let Ok(gemini_settings) = gemini_config_path(user) {
        crate::host_hooks::install_gemini_hooks(user, &gemini_settings, quiet)?;
    }
    if !quiet {
        println!("Installed Antigravity MCP at:");
        for path in &written {
            println!("  {}", path.display());
        }
        if user {
            println!("  User scope: ~/.gemini/antigravity/mcp_config.json (+ ~/.gemini/config/mcp_config.json for Antigravity 2.0)");
        } else {
            println!("  Project scope: .gemini/antigravity/mcp_config.json at repository root.");
        }
        println!(
            "  In Antigravity: Settings → Customizations → Refresh MCP servers (or /mcp in CLI)."
        );
        println!("  Route gate hooks: shared ~/.gemini/settings.json (BeforeAgent / BeforeTool).");
    }
    Ok(written)
}

fn install_gemini_instructions(user: bool, quiet: bool) -> Result<()> {
    let path = if user {
        let home = dirs::home_dir().context("home directory")?;
        let dir = home.join(".gemini");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    } else {
        let cwd = std::env::current_dir().context("current working directory")?;
        let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
        let dir = root.join(".gemini");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    };
    write_host_instructions(&path, HOST_AGENT_BRAIN_INSTRUCTIONS, quiet, "Gemini CLI")?;
    if let Some(parent) = path.parent() {
        write_agent_brain_mode(parent, quiet, "Gemini CLI")?;
    }
    Ok(())
}

fn install_antigravity_instructions(user: bool, quiet: bool) -> Result<()> {
    let path = if user {
        let home = dirs::home_dir().context("home directory")?;
        let dir = home.join(".gemini").join("antigravity");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    } else {
        let cwd = std::env::current_dir().context("current working directory")?;
        let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
        let dir = root.join(".gemini").join("antigravity");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    };
    write_host_instructions(&path, HOST_AGENT_BRAIN_INSTRUCTIONS, quiet, "Antigravity")?;
    if let Some(parent) = path.parent() {
        write_agent_brain_mode(parent, quiet, "Antigravity")?;
    }
    Ok(())
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

pub fn merge_opencode_config(path: &Path, server_entry: Value, user: bool) -> Result<Value> {
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

    let instruction_paths = opencode_instruction_paths(user);
    merge_json_string_array(&mut root, "instructions", &instruction_paths);
    merge_json_string_array(
        &mut root,
        "plugin",
        &[OPENCODE_PLUGIN_NAME.to_string()],
    );

    Ok(root)
}

fn opencode_instruction_paths(user: bool) -> Vec<String> {
    if user {
        vec![
            "~/.config/opencode/rules/agent-brain-route.md".into(),
            "~/.config/opencode/agent-brain.md".into(),
        ]
    } else {
        vec![
            ".opencode/rules/agent-brain-route.md".into(),
            ".opencode/agent-brain.md".into(),
        ]
    }
}

fn merge_json_string_array(root: &mut Value, key: &str, values: &[String]) {
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    if !obj.contains_key(key) {
        obj.insert(key.to_string(), json!([]));
    }
    let Some(arr) = obj.get_mut(key).and_then(|v| v.as_array_mut()) else {
        obj.insert(key.to_string(), json!(values));
        return;
    };
    for value in values {
        let entry = json!(value);
        if !arr.contains(&entry) {
            arr.push(entry);
        }
    }
}

const OPENCODE_PLUGIN_NAME: &str = "agent-brain-route-gate";

const OPENCODE_ROUTE_RULE: &str = r"## HARD RULES (never skip)

- Call `agent-brain_route_task` at the start of EVERY turn, before any other tool, skill, or action. Pass `user_message`, `current_working_directory`, and `open_files`. This is non-negotiable.
";

pub const AGENT_BRAIN_MODE_SNIPPET: &str = agent_body_core::BASE_AGENT_MODE;

fn write_agent_brain_mode(dir: &Path, quiet: bool, label: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join("agent-brain-mode.md");
    write_agent_brain_mode_file(&path, quiet, label)
}

pub fn write_agent_brain_mode_file(path: &Path, quiet: bool, label: &str) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, AGENT_BRAIN_MODE_SNIPPET)
        .with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!("Installed {label} agent-brain mode at {}", path.display());
    }
    Ok(path.to_path_buf())
}

pub fn write_cursor_agent_brain_mode_mdc(brain_home: &Path, quiet: bool) -> Result<PathBuf> {
    fs::create_dir_all(brain_home).with_context(|| format!("create {}", brain_home.display()))?;
    let path = brain_home.join("cursor-agent-brain-mode.mdc");
    let body = format!(
        "---\ndescription: agent-brain mode — delegate to Autonomic utilities\nalwaysApply: false\n---\n\n{AGENT_BRAIN_MODE_SNIPPET}",
    );
    fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!(
            "Installed Cursor agent-brain mode snippet at {} (paste into User Rules or enable per project)",
            path.display()
        );
    }
    Ok(path)
}

/// Canonical agent-brain mode file locations (global user scope).
pub fn agent_brain_mode_locations(user: bool) -> Vec<(&'static str, PathBuf)> {
    if !user {
        return Vec::new();
    }
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    let canonical = agent_body_core::agents_md_path();
    vec![
        ("cursor (User Rules)", home.join(".agent_brain/cursor-agent-brain-mode.mdc")),
        ("canonical", canonical.clone()),
        ("opencode", home.join(".config/opencode/AGENTS.md")),
        ("claude", home.join(".claude/AGENTS.md")),
        ("codex", home.join(".codex/AGENTS.md")),
        ("gemini", home.join(".gemini/AGENTS.md")),
        (
            "antigravity",
            home.join(".gemini/antigravity/AGENTS.md"),
        ),
        ("vscode", home.join(".vscode/AGENTS.md")),
    ]
}

/// Install agent-brain mode files for every supported host (idempotent).
pub fn install_agent_brain_modes(user: bool, quiet: bool) -> Result<()> {
    if !user {
        if !quiet {
            println!("agent-brain mode: project scope uses per-repo paths (.cursor/rules, .github/, etc.)");
        }
        return Ok(());
    }
    let home = dirs::home_dir().context("home directory")?;
    let brain_home = home.join(".agent_brain");

    agent_body_core::ensure_default_ecosystem_sections().ok();
    agent_body_core::scaffold_agents_dir().context("scaffold agents fragments")?;
    if let Some(content) = agent_body_core::default_fragment("brain") {
        agent_body_core::write_fragment("brain", content)?;
    }
    let links = agent_body_core::install_host_agents_md_links()?;
    let composed = agent_body_core::agents_md_path();

    write_cursor_agent_brain_mode_mdc(&brain_home, quiet)?;

    if !quiet {
        println!("Composed AGENTS.md at {}", composed.display());
        for link in &links {
            println!("  AGENTS.md link: {}", link.display());
        }
        println!();
        println!("agent-brain mode installed for all hosts. Run `agent-brain mode paths` to list files.");
        println!("  Cursor: paste ~/.agent_brain/cursor-user-rules.mdc AND cursor-agent-brain-mode.mdc into User Rules.");
    }
    Ok(())
}

pub fn print_agent_brain_mode_paths() {
    println!("agent-brain mode files (global / --global):\n");
    for (host, path) in agent_brain_mode_locations(true) {
        let status = if path.is_file() { "ok" } else { "missing" };
        println!("  {host:<14} {status:<7} {}", path.display());
    }
    println!("\nSnippet: `agent-brain mode show`");
}

fn write_opencode_config(path: &Path, server_entry: Value, user: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let merged = merge_opencode_config(path, server_entry, user)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(path, format!("{pretty}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn opencode_base_dir(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        let dir = home.join(".config").join("opencode");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    } else {
        let cwd = std::env::current_dir().context("current working directory")?;
        let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
        let dir = root.join(".opencode");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

fn install_opencode_instructions(user: bool, quiet: bool) -> Result<()> {
    let base = opencode_base_dir(user)?;
    let full_path = base.join("agent-brain.md");
    write_host_instructions(&full_path, HOST_AGENT_BRAIN_INSTRUCTIONS, quiet, "OpenCode")?;

    let rules_dir = base.join("rules");
    fs::create_dir_all(&rules_dir)?;
    fs::write(rules_dir.join("agent-brain-route.md"), OPENCODE_ROUTE_RULE)
        .with_context(|| format!("write {}", rules_dir.display()))?;

    let modes_dir = base.join("modes");
    fs::create_dir_all(&modes_dir)?;
    fs::write(modes_dir.join("agent-brain.md"), AGENT_BRAIN_MODE_SNIPPET)
        .with_context(|| format!("write {}", modes_dir.display()))?;

    if !quiet {
        println!(
            "Installed OpenCode route rule and agent-brain mode under {}",
            base.display()
        );
    }
    Ok(())
}

pub fn codex_config_path(user: bool) -> Result<PathBuf> {
    if user {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".codex").join("config.toml"));
    }
    let cwd = std::env::current_dir().context("current working directory")?;
    let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
    Ok(root.join(".codex").join("config.toml"))
}

pub fn codex_mcp_toml_block(exe: &Path) -> String {
    let entry = crate::install::mcp_server_entry(exe);
    let command = entry
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| exe.to_str().unwrap_or("agent-brain"));
    let args = entry
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(toml_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "\"serve\"".to_string());

    let env_pairs = entry
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| {
                    let owned = v.to_string();
                    let val = v.as_str().unwrap_or(&owned);
                    format!("{} = {}", k, toml_string(val))
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let mut block = format!(
        "{CODEX_MCP_MARKER}\n[mcp_servers.agent-brain]\ncommand = {}\nargs = [{args}]\nstartup_timeout_sec = 30",
        toml_string(command)
    );
    if !env_pairs.is_empty() {
        block.push_str(&format!("\nenv = {{ {env_pairs} }}"));
    }
    block
}

const CODEX_MCP_MARKER: &str = "# agent-brain MCP — managed by `agent-brain install --codex`";

fn toml_string(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | ':'))
    {
        format!("\"{value}\"")
    } else {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }
}

pub fn merge_codex_config_toml(content: &str, mcp_block: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if let Some((start, end)) = find_toml_section_range(&lines, "mcp_servers.agent-brain") {
        let mut out: Vec<String> = lines[..start]
            .to_vec()
            .into_iter()
            .map(str::to_string)
            .collect();
        if start > 0 && lines[start - 1].trim() == CODEX_MCP_MARKER {
            out.pop();
        }
        if !out.is_empty() && !out.last().map(|l| l.is_empty()).unwrap_or(true) {
            out.push(String::new());
        }
        out.extend(mcp_block.lines().map(str::to_string));
        if end < lines.len() {
            if !out.last().map(|l| l.is_empty()).unwrap_or(false) {
                out.push(String::new());
            }
            out.extend(lines[end..].iter().map(|l| (*l).to_string()));
        }
        let merged = out.join("\n");
        return normalize_trailing_newline(&merged);
    }

    let mut merged = content.trim_end().to_string();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(mcp_block);
    normalize_trailing_newline(&merged)
}

fn find_toml_section_range(lines: &[&str], section: &str) -> Option<(usize, usize)> {
    let header = format!("[{section}]");
    let start = lines.iter().position(|line| line.trim() == header)?;
    let end = lines[(start + 1)..]
        .iter()
        .position(|line| line.starts_with('[') && line.ends_with(']'))
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn normalize_trailing_newline(text: &str) -> String {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn write_codex_config(path: &Path, exe: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let existing = if path.is_file() {
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
    } else {
        String::new()
    };
    let merged = merge_codex_config_toml(&existing, &codex_mcp_toml_block(exe));
    fs::write(path, merged).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn install_codex(exe: &Path, user: bool, quiet: bool) -> Result<Vec<PathBuf>> {
    let config_path = codex_config_path(user)?;
    write_codex_config(&config_path, exe)?;
    install_codex_instructions(user, quiet)?;
    crate::host_hooks::install_codex_hooks(user, quiet)?;
    if !quiet {
        println!("Installed Codex MCP at {}", config_path.display());
        if user {
            println!("  User scope: ~/.codex/config.toml");
        } else {
            println!("  Project scope: .codex/config.toml at repository root.");
            println!("  Trust this project in Codex for project-local MCP to load.");
        }
        println!("  Route gate hooks: ~/.codex/hooks.json (or .codex/hooks.json).");
        println!("  Restart Codex or run `/hooks` to review and trust new hooks.");
        println!("  Verify with `codex mcp list` when available.");
    }
    Ok(vec![config_path])
}

fn install_codex_instructions(user: bool, quiet: bool) -> Result<()> {
    let path = if user {
        let home = dirs::home_dir().context("home directory")?;
        let dir = home.join(".codex");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    } else {
        let cwd = std::env::current_dir().context("current working directory")?;
        let root = crate::config::find_repo_root(&cwd).unwrap_or(cwd);
        let dir = root.join(".codex");
        fs::create_dir_all(&dir)?;
        dir.join("agent-brain.md")
    };
    write_host_instructions(&path, HOST_AGENT_BRAIN_INSTRUCTIONS, quiet, "Codex")?;
    if let Some(parent) = path.parent() {
        write_agent_brain_mode(parent, quiet, "Codex")?;
    }
    Ok(())
}

fn write_host_instructions(path: &Path, content: &str, quiet: bool, label: &str) -> Result<()> {
    let version_marker = format!("instructions-version: {HOST_INSTRUCTIONS_VERSION}");
    if path.is_file() {
        if let Ok(existing) = fs::read_to_string(path) {
            if existing.contains(&version_marker) {
                return Ok(());
            }
        }
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    if !quiet {
        println!("Installed {label} instructions at {}", path.display());
    }
    Ok(())
}

const HOST_INSTRUCTIONS_VERSION: &str = "6";

const HOST_AGENT_BRAIN_INSTRUCTIONS: &str = r#"# agent-brain MCP (required)
instructions-version: 6

## The connection contract

**route_task is the only entry point** for agent-brain context on every host.

- Session digests from Cursor, OpenCode, Codex, Gemini, and Antigravity are stored in brain.db but **only surface through route_task**.
- Team memory, skills, rules, and must_apply constraints are **only injected through route_task**.
- Other agent-brain MCP tools (`grep_search`, `store_memory`, `get_context`, …) **return errors** until route_task succeeds for this turn.
- **Install and route_task refresh session ingests** — cross-agent threads are indexed into brain.db automatically.

If the agent skips route_task, cross-agent ingest and shared memory provide **zero value**.

## Every user turn

1. Call **`route_task`** with `user_message`, `current_working_directory`, and `open_files`.
2. Load skills/agents from returned paths; apply `applicable_rules` and `must_apply`.
3. Use `relevant_memory` (includes session digests when relevant).
4. At task end, call **`store_memory`** for durable outcomes (max 50 words, no secrets).

## Autonomic utility ecosystem

When agent-brain mode is on, **delegate** to sibling utilities instead of improvising multi-step work:

| Utility | When | Invoke |
|---------|------|--------|
| **agent-spine** | Repeatable workflows (release notes, stacked PRs, bugfix loops) | `agent-spine run --meta "..."` or `agent-spine init --with @workflow` |
| **agent-heart** | Budget-sensitive or long runs | `agent-heart status` |
| **agent-body** | Route to any organ | `autonomic spine run`, `autonomic brain briefing` |

- `agent-brain registry list` — skill @aliases and utility catalog
- `route_to_mcp` — forward to configured upstream MCP tools (`suggested_tools` in route_task)
- Bootstrap: `agent-brain add @autonomic-core` + `agent-brain add @supervisor`

## Native host tools (OpenCode / Claude Code / VS Code / Gemini / Antigravity)

This host has **no hook gate** on Read/Shell/Grep. You must self-enforce:

- **Do not** use host Read/Cat/Grep for file exploration when agent-brain MCP is connected.
- **Use** agent-brain `grep_search`, `file_summary`, `read_file_head`, `read_file_tail` instead.
- Host native reads bypass routing, token savings, and cross-agent session digests.

On Cursor, hooks block host tools until route_task; on Codex and Claude Code, hooks gate MCP tools until route_task. On other hosts **you** must follow the same discipline.

## Continuing work from another IDE

When the user says "continue" or references work elsewhere:

1. Call **`route_task`** first — digests and memory may already describe the in-progress task.
2. Read **`agent-brain briefing`** or `~/.agent_brain/logs/last-route.md`.
3. Treat as in-progress work unless the user clearly changes direction.

Readable summary: `~/.agent_brain/logs/last-route.md` or `agent-brain briefing`.
"#;

const CLAUDE_CODE_AGENT_BRAIN_INSTRUCTIONS: &str = r#"# agent-brain MCP (required)
instructions-version: 6

## The connection contract

**route_task is the only entry point** for agent-brain context on every host.

- Session digests from Cursor, OpenCode, Codex, Gemini, and Antigravity are stored in brain.db but **only surface through route_task**.
- Team memory, skills, rules, and must_apply constraints are **only injected through route_task**.
- Other agent-brain MCP tools (`grep_search`, `store_memory`, `get_context`, …) **return errors** until route_task succeeds for this turn.
- **Install and route_task refresh session ingests** — cross-agent threads are indexed into brain.db automatically.

If the agent skips route_task, cross-agent ingest and shared memory provide **zero value**.

## Every user turn

1. Call **`route_task`** with `user_message`, `current_working_directory`, and `open_files`.
2. Load skills/agents from returned paths; apply `applicable_rules` and `must_apply`.
3. Use `relevant_memory` (includes session digests when relevant).
4. At task end, call **`store_memory`** for durable outcomes (max 50 words, no secrets).

## Claude Code enforcement (hooks)

`agent-brain install --claude-code [--global]` installs route gate hooks in `.claude/settings.json`:

- **UserPromptSubmit** — marks each new prompt as needing `route_task`.
- **PreToolUse** (`mcp__agent-brain__.*`) — blocks other agent-brain MCP tools until `route_task` succeeds.
- **PostToolUse** — clears the gate after a successful `mcp__agent-brain__route_task`.

Native Claude tools (Read, Bash, Grep, …) are not blocked by the gate. Prefer agent-brain `grep_search`, `file_summary`, `read_file_head`, and `read_file_tail` for exploration.

Run `/hooks` to verify hooks are trusted. Re-run `agent-brain install --claude-code --global` or `agent-brain doctor --fix` if enforcement stops working.

## Continuing work from another IDE

When the user says "continue" or references work elsewhere:

1. Call **`route_task`** first — digests and memory may already describe the in-progress task.
2. Read **`agent-brain briefing`** or `~/.agent_brain/logs/last-route.md`.
3. Treat as in-progress work unless the user clearly changes direction.

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
    write_host_instructions(
        &path,
        CLAUDE_CODE_AGENT_BRAIN_INSTRUCTIONS,
        quiet,
        "Claude Code",
    )?;
    if let Some(parent) = path.parent() {
        write_agent_brain_mode(parent, quiet, "Claude Code")?;
    }
    Ok(())
}

pub fn merge_claude_json_mcp(path: &Path, server_entry: Value) -> Result<Value> {
    let mut root = if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) if v.is_object() => v,
            Ok(_) => {
                eprintln!(
                    "warning: {} is not a JSON object — overwriting",
                    path.display()
                );
                json!({})
            }
            Err(e) => {
                eprintln!(
                    "warning: {} parse error ({e}) — overwriting",
                    path.display()
                );
                json!({})
            }
        }
    } else {
        json!({})
    };

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
    crate::host_hooks::install_claude_code_hooks(user, quiet)?;
    if !quiet {
        println!("Installed Claude Code MCP at {}", path.display());
        if user {
            println!(
                "  User scope: ~/.claude.json (not settings.json — that file ignores mcpServers)."
            );
        } else {
            println!("  Project scope: .mcp.json at repository root.");
        }
        println!("  Start a new Claude Code session; run /mcp to verify.");
        println!("  Route gate hooks: UserPromptSubmit + PreToolUse/PostToolUse on mcp__agent-brain__.* in .claude/settings.json.");
        println!("  Trust hooks with /hooks if Claude Code prompts you.");
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
            true,
        )
        .unwrap();
        assert_eq!(merged["model"], "test/model");
        assert_eq!(merged["mcp"]["agent-brain"]["type"], "local");
        let instructions = merged["instructions"].as_array().unwrap();
        assert!(instructions.iter().any(|v| {
            v.as_str()
                .is_some_and(|s| s.contains("agent-brain-route.md"))
        }));
        assert!(instructions.iter().any(|v| {
            v.as_str().is_some_and(|s| s.contains("agent-brain.md"))
        }));
        let plugins = merged["plugin"].as_array().unwrap();
        assert!(plugins
            .iter()
            .any(|v| v.as_str() == Some("agent-brain-route-gate")));
    }

    fn host_target_parses_opencode_flag() {
        let args = vec!["install".into(), "--opencode".into(), "--global".into()];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::OpenCode { user: true }
        );
    }

    #[test]
    fn host_target_parses_gemini_flag() {
        let args = vec!["install".into(), "--gemini".into(), "--global".into()];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::Gemini { user: true }
        );
    }

    #[test]
    fn host_target_parses_antigravity_flag() {
        let args = vec!["install".into(), "--antigravity".into(), "--global".into()];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::Antigravity { user: true }
        );
    }

    #[test]
    fn gemini_project_uses_settings_json() {
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let path = gemini_config_path(false).unwrap();
        assert!(path.ends_with(".gemini/settings.json"));
    }

    #[test]
    fn antigravity_global_paths_include_shared_config() {
        let paths = antigravity_config_paths(true).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0]
            .to_string_lossy()
            .contains("antigravity/mcp_config.json"));
        assert!(paths[1]
            .to_string_lossy()
            .contains("config/mcp_config.json"));
    }

    #[test]
    fn merges_gemini_settings_preserves_other_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
        let merged = merge_claude_json_mcp(
            &path,
            json!({"command":"/bin/agent-brain","args":["serve"]}),
        )
        .unwrap();
        assert!(merged["mcpServers"]["agent-brain"].is_object());
        assert_eq!(merged["theme"], "dark");
    }

    #[test]
    fn host_target_parses_codex_flag() {
        let args = vec!["install".into(), "--codex".into(), "--global".into()];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::Codex { user: true }
        );
    }

    #[test]
    fn merges_codex_config_appends_when_missing() {
        let block = codex_mcp_toml_block(Path::new("/bin/agent-brain"));
        let merged = merge_codex_config_toml("model = \"gpt-5\"\n", &block);
        assert!(merged.contains("model = \"gpt-5\""));
        assert!(merged.contains("[mcp_servers.agent-brain]"));
        assert!(merged.contains("command = \"/bin/agent-brain\""));
    }

    #[test]
    fn merges_codex_config_replaces_existing_section() {
        let block = codex_mcp_toml_block(Path::new("/new/agent-brain"));
        let existing = r#"# agent-brain MCP — managed by `agent-brain install --codex`
[mcp_servers.agent-brain]
command = "/old/agent-brain"
args = ["serve"]

[profiles.strict]
approval_policy = "on-request"
"#;
        let merged = merge_codex_config_toml(existing, &block);
        assert!(merged.contains("/new/agent-brain"));
        assert!(!merged.contains("/old/agent-brain"));
        assert!(merged.contains("[profiles.strict]"));
    }

    #[test]
    fn host_target_parses_flags() {
        let args = vec!["install".into(), "--claude-code".into(), "--global".into()];
        assert_eq!(
            HostTarget::from_args(&args),
            HostTarget::ClaudeCode { user: true }
        );
    }

    #[test]
    fn agent_brain_mode_locations_include_all_hosts() {
        let locs = agent_brain_mode_locations(true);
        let hosts: Vec<_> = locs.iter().map(|(h, _)| *h).collect();
        assert!(hosts.iter().any(|h| h.contains("cursor")));
        assert!(hosts.iter().any(|h| *h == "codex"));
        assert!(hosts.iter().any(|h| *h == "opencode"));
        assert_eq!(locs.len(), 8);
    }
}
