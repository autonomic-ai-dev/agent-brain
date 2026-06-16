use agent_brain::host_install::{self, HostTarget};
use agent_brain::install::{merge_mcp_config, mcp_server_entry};
use std::path::Path;
use tempfile::TempDir;

#[test]
fn install_flags_resolve_hosts() {
    assert_eq!(
        HostTarget::from_args(&["install".into(), "--global".into()]),
        HostTarget::Cursor { global: true }
    );
    assert_eq!(
        HostTarget::from_args(&["install".into(), "--claude-desktop".into()]),
        HostTarget::ClaudeDesktop
    );
    assert_eq!(
        HostTarget::from_args(&["install".into(), "--all".into()]),
        HostTarget::All
    );
}

#[test]
fn claude_desktop_path_is_absolute_file() {
    let path = host_install::claude_desktop_config_path().unwrap();
    assert!(path.to_string_lossy().contains("claude_desktop_config.json"));
}

#[test]
fn vscode_workspace_path_under_vscode_dir() {
    let dir = TempDir::new().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let path = host_install::vscode_mcp_path(false).unwrap();
    assert!(path.ends_with(".vscode/mcp.json"));
}

#[test]
fn claude_code_project_uses_mcp_json_at_root() {
    let dir = TempDir::new().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let path = host_install::claude_code_mcp_path(false).unwrap();
    assert!(path.file_name().unwrap().to_string_lossy().ends_with("mcp.json"));
}

#[test]
fn merge_preserves_other_mcp_servers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mcp.json");
    std::fs::write(
        &path,
        r#"{"mcpServers":{"other":{"command":"other"}}}"#,
    )
    .unwrap();
    let merged = merge_mcp_config(
        &path,
        mcp_server_entry(Path::new("/usr/local/bin/agent-brain")),
    )
    .unwrap();
    assert!(merged["mcpServers"]["agent-brain"].is_object());
    assert!(merged["mcpServers"]["other"].is_object());
}
