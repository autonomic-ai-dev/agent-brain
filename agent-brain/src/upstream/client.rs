use anyhow::{Context, Result};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::db::store::BrainStore;
use crate::settings::{UpstreamMcpSettings, UpstreamServerConfig};
use crate::upstream::{
    enabled_servers, resolve_env_map, secret_names_from_env, validate_server_config,
    IndexedUpstreamTool,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamCallLog {
    pub server: String,
    pub tool: String,
    pub tokens_used: usize,
    pub truncated: bool,
    pub is_error: bool,
}

pub async fn refresh_upstream_index(
    settings: &UpstreamMcpSettings,
    store: &BrainStore,
) -> Result<usize> {
    if !settings.enabled {
        store.replace_upstream_tools(&[])?;
        return Ok(0);
    }

    let mut indexed = Vec::new();
    for server in enabled_servers(settings) {
        validate_server_config(server)?;
        register_secret_refs(store, server)?;
        match list_server_tools(server).await {
            Ok(mut tools) => indexed.append(&mut tools),
            Err(err) => {
                tracing::warn!(server = %server.name, error = %err, "upstream list_tools failed");
            }
        }
    }
    store.replace_upstream_tools(&indexed)?;
    Ok(indexed.len())
}

fn register_secret_refs(store: &BrainStore, server: &UpstreamServerConfig) -> Result<()> {
    for name in secret_names_from_env(&server.env) {
        let used_by = format!("upstream_mcp.{}", server.name);
        store.upsert_secret_ref(&name, &used_by)?;
    }
    Ok(())
}

async fn list_server_tools(server: &UpstreamServerConfig) -> Result<Vec<IndexedUpstreamTool>> {
    let service = connect(server).await?;
    let tools = service.list_all_tools().await?;
    service.cancel().await.ok();
    Ok(tools
        .into_iter()
        .map(|tool| IndexedUpstreamTool {
            server: server.name.clone(),
            name: tool.name.to_string(),
            description: tool.description.unwrap_or_default().to_string(),
        })
        .collect())
}

pub fn refresh_upstream_index_blocking(
    settings: &UpstreamMcpSettings,
    store: &BrainStore,
) -> Result<usize> {
    // Spawn in a fresh OS thread via scoped thread to avoid nested
    // tokio runtime panic.  The add/update CLI handlers call this
    // from within #[tokio::main], where Runtime::new() would panic
    // because tokio forbids creating a runtime inside a running
    // runtime.  A scoped thread has no existing tokio context.
    let settings = settings.clone();
    let result = std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Runtime::new()
                .context("create tokio runtime for upstream index")?;
            rt.block_on(refresh_upstream_index(&settings, store))
        })
        .join()
    });
    match result {
        Ok(Ok(count)) => Ok(count),
        Ok(Err(err)) => Err(err),
        Err(e) => anyhow::bail!("upstream index thread panicked: {:?}", e),
    }
}

pub async fn call_upstream_tool(
    server: &UpstreamServerConfig,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<CallToolResult> {
    validate_server_config(server)?;
    let service = connect(server).await?;
    let mut req = CallToolRequestParams::new(tool_name.to_string());
    if let Some(args) = arguments.as_object().cloned() {
        req = req.with_arguments(args);
    }
    let result = service
        .call_tool(req)
        .await
        .context("upstream tools/call failed")?;
    service.cancel().await.ok();
    Ok(result)
}

async fn connect(
    server: &UpstreamServerConfig,
) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ()>> {
    let env = resolve_env_map(&server.env)?;
    let mut command = Command::new(&server.command);
    command.args(&server.args);
    for (key, value) in env {
        command.env(key, value);
    }
    let transport =
        TokioChildProcess::new(command.configure(|_| {})).context("spawn upstream MCP process")?;
    ().serve(transport)
        .await
        .map_err(|e| anyhow::anyhow!("upstream MCP initialize failed: {e}"))
}

pub fn call_tool_result_to_text(result: &CallToolResult) -> (String, Option<serde_json::Value>) {
    if let Some(structured) = &result.structured_content {
        return (structured.to_string(), Some(structured.clone()));
    }
    let mut parts = Vec::new();
    for content in &result.content {
        if let Some(text) = content.as_text() {
            parts.push(text.text.clone());
        } else {
            parts.push(serde_json::to_string(content).unwrap_or_else(|_| format!("{content:?}")));
        }
    }
    let joined = parts.join("\n");
    let structured = serde_json::from_str::<serde_json::Value>(&joined).ok();
    (joined, structured)
}
