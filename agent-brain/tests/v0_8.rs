use agent_brain::config::Config;
use agent_brain::db::store::BrainStore;
use agent_brain::settings::{AgentBrainSettings, UpstreamMcpSettings};
use agent_brain::upstream::{
    resolve_env_value, secret_names_from_env, suggest_upstream_tools, truncate_upstream_result,
    IndexedUpstreamTool,
};
use std::collections::HashMap;
use tempfile::TempDir;

fn test_config(home: &std::path::Path) -> Config {
    let mut config = Config::load().unwrap();
    config.home = home.to_path_buf();
    config.data_dir = home.join("data");
    config.logs_dir = home.join("logs");
    config.db_path = config.data_dir.join("brain.db");
    config.vectors_path = config.data_dir.join("vectors.bin");
    config
}

#[test]
fn upstream_env_template_extracts_secret_names() {
    let mut env = HashMap::new();
    env.insert("TOKEN".into(), "${GITHUB_TOKEN}".into());
    assert_eq!(
        secret_names_from_env(&env),
        vec!["GITHUB_TOKEN".to_string()]
    );
}

#[test]
fn upstream_env_resolution_uses_env_fallback() {
    std::env::set_var("AGENT_BRAIN_UPSTREAM_TEST", "resolved");
    assert_eq!(
        resolve_env_value("${AGENT_BRAIN_UPSTREAM_TEST}").unwrap(),
        "resolved"
    );
    std::env::remove_var("AGENT_BRAIN_UPSTREAM_TEST");
}

#[test]
fn suggest_tools_from_indexed_catalog() {
    let dir = TempDir::new().unwrap();
    let config = test_config(dir.path());
    config.ensure_dirs().unwrap();
    let store = BrainStore::open(&config.db_path).unwrap();
    store
        .replace_upstream_tools(&[IndexedUpstreamTool {
            server: "github".into(),
            name: "search_issues".into(),
            description: "Search GitHub issues".into(),
        }])
        .unwrap();
    let settings = UpstreamMcpSettings {
        enabled: true,
        servers: vec![agent_brain::settings::UpstreamServerConfig {
            name: "github".into(),
            command: "echo".into(),
            args: vec![],
            env: HashMap::new(),
            enabled: true,
        }],
        ..UpstreamMcpSettings::default()
    };
    let suggested = suggest_upstream_tools(&store, &settings, "search github issues in repo", 2);
    assert!(!suggested.is_empty());
    assert_eq!(suggested[0].server, "github");
}

#[test]
fn semantic_truncation_keeps_valid_json() {
    let items: Vec<serde_json::Value> = (0..50)
        .map(|i| serde_json::json!({"id": i, "title": format!("item-{i}")}))
        .collect();
    let raw = serde_json::to_string(&items).unwrap();
    let truncated = truncate_upstream_result(&raw, None, 60).unwrap();
    assert!(truncated.truncated);
    assert!(truncated.content.is_array());
    serde_json::to_string(&truncated.content).expect("valid json");
}

#[test]
fn settings_yaml_parses_upstream_block() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("config.yaml"),
        r#"
upstream_mcp:
  enabled: true
  servers:
    - name: demo
      command: echo
"#,
    )
    .unwrap();
    let settings = AgentBrainSettings::from_file(dir.path()).unwrap();
    assert!(settings.upstream_mcp.enabled);
    assert_eq!(settings.upstream_mcp.servers[0].name, "demo");
}
