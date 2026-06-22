use agent_brain::host_install::{self, HostTarget};
use agent_brain::install::{mcp_server_entry, merge_mcp_config};
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
    assert!(path
        .to_string_lossy()
        .contains("claude_desktop_config.json"));
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
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("mcp.json"));
}

#[test]
fn opencode_project_uses_opencode_json_at_root() {
    let dir = TempDir::new().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let path = host_install::opencode_config_path(false).unwrap();
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("opencode.json"));
}

#[test]
fn merge_opencode_preserves_other_keys() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("opencode.json");
    std::fs::write(
        &path,
        r#"{"$schema":"https://opencode.ai/config.json","model":"test/model"}"#,
    )
    .unwrap();
    let merged = host_install::merge_opencode_config(
        &path,
        serde_json::json!({
            "type": "local",
            "command": ["/bin/agent-brain", "serve"],
            "enabled": true
        }),
        false,
    )
    .unwrap();
    assert_eq!(merged["model"], "test/model");
    assert!(merged["mcp"]["agent-brain"].is_object());
}

#[test]
fn install_flags_resolve_opencode() {
    assert_eq!(
        HostTarget::from_args(&["install".into(), "--opencode".into(), "--global".into()]),
        HostTarget::OpenCode { user: true }
    );
}

#[test]
fn install_flags_resolve_gemini() {
    assert_eq!(
        HostTarget::from_args(&["install".into(), "--gemini".into(), "--global".into()]),
        HostTarget::Gemini { user: true }
    );
}

#[test]
fn install_flags_resolve_antigravity() {
    assert_eq!(
        HostTarget::from_args(&["install".into(), "--antigravity".into(), "--global".into()]),
        HostTarget::Antigravity { user: true }
    );
}

#[test]
fn gemini_project_uses_settings_json_at_root() {
    let dir = TempDir::new().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let path = host_install::gemini_config_path(false).unwrap();
    assert!(path.ends_with(".gemini/settings.json"));
}

#[test]
fn merge_preserves_other_mcp_servers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mcp.json");
    std::fs::write(&path, r#"{"mcpServers":{"other":{"command":"other"}}}"#).unwrap();
    let merged = merge_mcp_config(
        &path,
        mcp_server_entry(Path::new("/usr/local/bin/agent-brain")),
    )
    .unwrap();
    assert!(merged["mcpServers"]["agent-brain"].is_object());
    assert!(merged["mcpServers"]["other"].is_object());
}
