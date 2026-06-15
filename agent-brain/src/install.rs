use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn run(global: bool, print_only: bool) -> Result<()> {
    let exe = std::env::current_exe().context("resolve agent-brain binary path")?;
    let config_path = mcp_config_path(global)?;
    let snippet = mcp_server_entry(&exe);

    if print_only {
        println!("{}", serde_json::to_string_pretty(&json!({ "mcpServers": { "agent-brain": snippet } }))?);
        return Ok(());
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let merged = merge_mcp_config(&config_path, snippet)?;
    let pretty = serde_json::to_string_pretty(&merged)?;
    fs::write(&config_path, format!("{pretty}\n")).with_context(|| {
        format!("write MCP config to {}", config_path.display())
    })?;

    println!("agent-brain MCP configured at {}", config_path.display());
    println!("Binary: {}", exe.display());
    println!();
    println!("Next steps:");
    println!("  1. Restart Cursor (or reload MCP in Settings → MCP)");
    println!("  2. Confirm 'agent-brain' appears and is enabled");
    println!("  3. Run once: agent-brain index");
    Ok(())
}

fn mcp_config_path(global: bool) -> Result<PathBuf> {
    if global {
        let home = dirs::home_dir().context("home directory")?;
        return Ok(home.join(".cursor").join("mcp.json"));
    }
    Ok(std::env::current_dir()?.join(".cursor").join("mcp.json"))
}

fn mcp_server_entry(exe: &Path) -> Value {
    json!({
        "command": exe.display().to_string(),
        "args": ["serve"],
        "env": {
            "RUST_LOG": "agent_brain=info"
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
