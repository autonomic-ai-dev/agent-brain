use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::store::{content_hash, looks_like_secret, word_count};
use crate::db::write_queue::{store_memory_payload, WriteOp, WriteQueue};
use crate::engine::Engine;
use crate::types::{deserialize_route_limits, ItemType, RouteLimits};

#[derive(Clone)]
pub struct BrainMcp {
    engine: Arc<Engine>,
    write_queue: Arc<WriteQueue>,
    tool_router: ToolRouter<Self>,
}

impl BrainMcp {
    pub fn new(engine: Arc<Engine>) -> Self {
        let store = engine.store.clone();
        let embedder = engine.embedder.clone();
        let cache = engine.cache.clone();
        let auto = engine.auto_capture_enabled;
        let home = engine.config.home.clone();

        let write_queue = WriteQueue::spawn(move |op| match op {
            WriteOp::StoreMemory { resp_tx, payload } => {
                let result = (|| {
                    if !auto {
                        anyhow::bail!("auto capture disabled");
                    }
                    if looks_like_secret(&payload.fact) {
                        anyhow::bail!("prohibited content");
                    }
                    if word_count(&payload.fact) > 50 {
                        anyhow::bail!("fact exceeds 50 words");
                    }
                    let hash = content_hash(&payload.fact);
                    let embedding = embedder.embed_one(&format!("{} {}", payload.topic, payload.fact))?;
                    let polarity = payload.polarity.as_deref().unwrap_or("positive");
                    let apply_when = payload
                        .apply_when
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?;
                    let res = store.store_fact_full(
                        &payload.topic,
                        &payload.fact,
                        &payload.scope,
                        payload.scope_key.as_deref(),
                        payload.confidence,
                        "agent",
                        &hash,
                        &embedding,
                        polarity,
                        apply_when.as_deref(),
                    )?;
                    cache.clear();
                    store.bump_index_version().ok();

                    if res.stored {
                        let settings = crate::settings::AgentBrainSettings::load(&home);
                        if settings.sync.git.auto_push {
                            if let Err(err) =
                                crate::sync::git_push(&store, &home, &settings.sync.git)
                            {
                                tracing::warn!(target: "agent_brain::sync", "auto_push failed: {err}");
                            }
                        }
                    }

                    Ok(serde_json::json!({
                        "id": res.id,
                        "stored": res.stored,
                        "deduplicated": res.deduplicated
                    }))
                })();
                let _ = resp_tx.send(result);
            }
            WriteOp::DeleteMemory {
                resp_tx,
                id,
                topic,
                scope,
                scope_key,
            } => {
                let result = store
                    .delete_fact(
                        id.as_deref(),
                        topic.as_deref(),
                        scope.as_deref(),
                        scope_key.as_deref(),
                    )
                    .map(|n| serde_json::json!({ "deleted": n }));
                let _ = resp_tx.send(result);
            }
            WriteOp::ReindexComplete => {}
        });

        Self {
            engine,
            write_queue: Arc::new(write_queue),
            tool_router: Self::tool_router(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RouteTaskParams {
    user_message: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default)]
    open_files: Vec<String>,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
    #[serde(default = "default_route_limits", deserialize_with = "deserialize_route_limits")]
    #[schemars(default = "default_route_limits")]
    limits: RouteLimits,
    #[serde(default)]
    phase: Option<String>,
}

fn default_route_limits() -> RouteLimits {
    RouteLimits::default()
}

fn default_max_tokens() -> usize {
    500
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetContextParams {
    task_description: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default = "default_ctx_tokens")]
    max_tokens: usize,
    #[serde(default = "default_include_types")]
    include_types: Vec<String>,
}

fn default_ctx_tokens() -> usize {
    300
}

fn default_include_types() -> Vec<String> {
    vec![
        "rule".into(),
        "skill".into(),
        "agent".into(),
        "memory".into(),
    ]
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StoreMemoryParams {
    topic: String,
    fact: String,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    polarity: Option<String>,
    #[serde(default)]
    apply_when: Option<Vec<String>>,
}

fn default_scope() -> String {
    "project".into()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListMemoryParams {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteMemoryParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scope_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExportMemoryParams {
    #[serde(default)]
    filename: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReportContextUsefulParams {
    item_ids: Vec<String>,
    useful: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExplainLastContextParams {
    #[serde(default)]
    log_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ImportMemoryParams {
    bundle_path: String,
    #[serde(default = "default_merge_policy")]
    merge_policy: String,
}

fn default_merge_policy() -> String {
    "newer_wins".into()
}

#[tool_router]
impl BrainMcp {
    #[tool(description = "REQUIRED every turn before planning or edits. Returns ranked agents, skills, rules, and memory under a token budget. Pass user_message, current_working_directory, and open_files.")]
    async fn route_task(
        &self,
        params: Parameters<RouteTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p
            .current_working_directory
            .as_ref()
            .map(PathBuf::from);
        let resp = self
            .engine
            .route_task(
                &p.user_message,
                cwd.as_deref(),
                &p.open_files,
                p.max_tokens,
                p.limits.normalize(),
                p.phase.as_deref(),
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(resp)
    }

    #[tool(description = "Lower-level flat context retrieval.")]
    async fn get_context(
        &self,
        params: Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p
            .current_working_directory
            .as_ref()
            .map(PathBuf::from);
        let types: Vec<ItemType> = p
            .include_types
            .iter()
            .filter_map(|s| ItemType::parse(s))
            .collect();
        let resp = self
            .engine
            .get_context(
                &p.task_description,
                cwd.as_deref(),
                p.max_tokens,
                &types,
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(resp)
    }

    #[tool(description = "REQUIRED at task end for durable decisions. Max 50 words. No secrets.")]
    async fn store_memory(
        &self,
        params: Parameters<StoreMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        if looks_like_secret(&p.fact) {
            return Err(McpError::invalid_params(
                "prohibited content detected",
                None,
            ));
        }
        if word_count(&p.fact) > 50 {
            return Err(McpError::invalid_params("fact exceeds 50 words", None));
        }

        let scope_key = std::env::current_dir()
            .ok()
            .and_then(|c| crate::config::find_repo_root(&c))
            .map(|p| p.display().to_string());

        let polarity = p.polarity.or_else(|| infer_memory_polarity(&p.fact));

        let (tx, rx) = std::sync::mpsc::channel();
        self.write_queue
            .send(WriteOp::StoreMemory {
                resp_tx: tx,
                payload: store_memory_payload::StoreMemoryRequest {
                    topic: p.topic,
                    fact: p.fact,
                    scope: p.scope,
                    scope_key,
                    confidence: if p.confidence == 0.0 { 0.9 } else { p.confidence },
                    polarity,
                    apply_when: p.apply_when,
                },
            })
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

        let value = rx
            .recv()
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(value)
    }

    #[tool(description = "List stored memory facts.")]
    async fn list_memory(
        &self,
        params: Parameters<ListMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let facts = self
            .engine
            .store
            .list_facts(params.0.limit)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({ "facts": facts }))
    }

    #[tool(description = "Delete a stored fact by id or topic+scope.")]
    async fn delete_memory(
        &self,
        params: Parameters<DeleteMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let (tx, rx) = std::sync::mpsc::channel();
        self.write_queue
            .send(WriteOp::DeleteMemory {
                resp_tx: tx,
                id: p.id,
                topic: p.topic,
                scope: p.scope,
                scope_key: p.scope_key,
            })
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = rx
            .recv()
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(value)
    }

    #[tool(description = "Export memory facts to ~/.agent_brain/export/")]
    async fn export_memory(
        &self,
        params: Parameters<ExportMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let filename = params.0.filename.unwrap_or_else(|| {
            format!("export-{}.json", chrono::Utc::now().timestamp())
        });
        let path = self.engine.config.home.join("export").join(filename);
        let written = self
            .engine
            .store
            .export_facts(&path)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({ "path": written }))
    }

    #[tool(description = "Report whether retrieved context items were useful (feedback loop).")]
    async fn report_context_useful(
        &self,
        params: Parameters<ReportContextUsefulParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let updated = self
            .engine
            .store
            .record_context_feedback(&p.item_ids, p.useful)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({
            "updated": updated,
            "useful": p.useful,
            "reason": p.reason
        }))
    }

    #[tool(description = "Explain the last route_task retrieval (or a specific log_id).")]
    async fn explain_last_context(
        &self,
        params: Parameters<ExplainLastContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let explain = crate::observability::explain_last(
            &self.engine.store,
            params.0.log_id.as_deref(),
        )
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({ "context": explain }))
    }

    #[tool(description = "Import memory facts from a sync bundle directory.")]
    async fn import_memory(
        &self,
        params: Parameters<ImportMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let policy = crate::sync::MergePolicy::parse(&p.merge_policy)
            .ok_or_else(|| McpError::invalid_params("invalid merge_policy", None))?;
        let report = crate::sync::import_bundle(
            &self.engine.store,
            &self.engine.embedder,
            PathBuf::from(&p.bundle_path).as_path(),
            policy,
            crate::sync::SyncSource::ManualImport,
        )
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        self.engine.store.bump_index_version().ok();
        self.engine.cache.clear();
        json_result(report)
    }
}

#[tool_handler]
impl ServerHandler for BrainMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "agent-brain is the routing layer for this session. \
                 REQUIRED: call route_task at the start of every user turn before choosing skills, rules, or agents. \
                 Use returned paths to load skills; apply applicable_rules and must_apply. \
                 At task end, call store_memory for durable outcomes (max 50 words). \
                 Do not bypass this server when its tools are available."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "agent-brain".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

fn json_result<T: serde::Serialize>(value: T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string(&value)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn infer_memory_polarity(fact: &str) -> Option<String> {
    let lower = fact.to_lowercase();
    if lower.contains("do not") || lower.starts_with("never ") || lower.contains("never use") {
        Some("negative".into())
    } else {
        None
    }
}

pub async fn run_stdio(engine: Arc<Engine>) -> Result<()> {
    let server = BrainMcp::new(engine);
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
