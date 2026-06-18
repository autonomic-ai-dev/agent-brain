use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::store::{looks_like_secret, word_count};
use crate::db::write_queue::{store_memory_payload, WriteOp};
use crate::engine::Engine;
use crate::mcp::route_gate::McpRouteGate;
use crate::types::{deserialize_route_limits, ItemType, RouteLimits};

#[derive(Clone)]
pub struct BrainMcp {
    engine: Arc<Engine>,
    tool_router: ToolRouter<Self>,
    route_gate: Arc<McpRouteGate>,
}

impl BrainMcp {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            route_gate: Arc::new(McpRouteGate::from_config(&engine.config)),
            engine,
            tool_router: Self::tool_router(),
        }
    }

    fn require_route(&self, tool: &str) -> Result<(), McpError> {
        self.route_gate.require_route(tool)
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

#[derive(Debug, Deserialize, JsonSchema)]
struct RouteToMcpParams {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: serde_json::Value,
    #[serde(default = "default_upstream_tokens")]
    max_tokens: usize,
}

fn default_upstream_tokens() -> usize {
    500
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PromoteToSkillParams {
    #[serde(default)]
    fact_id: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    skill_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TokenToolPathParams {
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default)]
    allow_blocked_paths: bool,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadFileBoundedParams {
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default = "default_read_lines")]
    lines: usize,
    #[serde(default = "default_read_bytes")]
    max_bytes: usize,
    #[serde(default)]
    allow_blocked_paths: bool,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

fn default_read_lines() -> usize {
    crate::token_tools::DEFAULT_MAX_LINES
}

fn default_read_bytes() -> usize {
    crate::token_tools::DEFAULT_MAX_BYTES
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GrepSearchParams {
    pattern: String,
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default = "default_grep_matches")]
    max_matches: usize,
    #[serde(default)]
    case_insensitive: bool,
    #[serde(default)]
    allow_blocked_paths: bool,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

fn default_grep_matches() -> usize {
    crate::token_tools::DEFAULT_GREP_MAX_MATCHES
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryCodebaseParams {
    question: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default = "default_query_budget")]
    max_tokens: usize,
}

fn default_query_budget() -> usize {
    1500
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TriggerDeepAnalysisParams {
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphifyJobStatusParams {
    job_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LearnFromUrlParams {
    url: String,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

#[tool_router]
impl BrainMcp {
    #[tool(description = "REQUIRED every turn before planning or edits. Returns ranked agents, skills, rules, and memory under a token budget. Pass user_message, current_working_directory, and open_files. Session digests from Cursor/OpenCode/Codex/Gemini/Antigravity and team memory are only injected here.")]
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
        self.route_gate.record_route(&p.user_message);
        json_result(resp)
    }

    #[tool(description = "Lower-level flat context retrieval. Requires route_task first in this turn.")]
    async fn get_context(
        &self,
        params: Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("get_context")?;
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

    #[tool(description = "REQUIRED at task end for durable decisions. Max 50 words. No secrets. Requires route_task first in this turn.")]
    async fn store_memory(
        &self,
        params: Parameters<StoreMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("store_memory")?;
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
        self.engine
            .write_queue()
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
        self.require_route("list_memory")?;
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
        self.require_route("delete_memory")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let (tx, rx) = std::sync::mpsc::channel();
        self.engine
            .write_queue()
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
        self.require_route("export_memory")?;
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
        self.require_route("report_context_useful")?;
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
        self.require_route("explain_last_context")?;
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
        self.require_route("import_memory")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let policy = crate::sync::MergePolicy::parse(&p.merge_policy)
            .ok_or_else(|| McpError::invalid_params("invalid merge_policy", None))?;
        let report = self
            .engine
            .import_bundle_queued(
                PathBuf::from(&p.bundle_path).as_path(),
                policy,
                crate::sync::SyncSource::ManualImport,
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        self.engine.cache.clear();
        json_result(report)
    }

    #[tool(description = "Call a configured upstream MCP tool. Response is semantically truncated to max_tokens. Agent must call explicitly; router never auto-executes upstream tools.")]
    async fn route_to_mcp(
        &self,
        params: Parameters<RouteToMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("route_to_mcp")?;
        let started = std::time::Instant::now();
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let settings = crate::settings::AgentBrainSettings::load(&self.engine.config.home);
        let server = crate::upstream::find_server(&settings.upstream_mcp, &p.server).ok_or_else(|| {
            McpError::invalid_params(
                format!("unknown or disabled upstream server: {}", p.server),
                None,
            )
        })?;
        let arguments = if p.arguments.is_null() {
            serde_json::json!({})
        } else {
            p.arguments
        };
        let result = crate::upstream::call_upstream_tool(server, &p.tool, arguments)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let is_error = result.is_error.unwrap_or(false);
        let (raw, structured) = crate::upstream::call_tool_result_to_text(&result);
        let truncated = crate::upstream::truncate_upstream_result(
            &raw,
            structured.as_ref(),
            p.max_tokens,
        )
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let log_id = uuid::Uuid::new_v4().to_string();
        let latency_ms = started.elapsed().as_millis() as u64;
        if let Err(err) = crate::observability::log_upstream_call(
            &self.engine.store,
            &log_id,
            &server.name,
            &p.tool,
            truncated.tokens_used,
            truncated.truncated,
            is_error,
            latency_ms,
        ) {
            tracing::warn!(error = %err, "upstream retrieval log write failed");
        }
        json_result(serde_json::json!({
            "log_id": log_id,
            "server": server.name,
            "tool": p.tool,
            "is_error": is_error,
            "truncated": truncated.truncated,
            "tokens_used": truncated.tokens_used,
            "tokens_budget": truncated.tokens_budget,
            "content": truncated.content,
        }))
    }

    #[tool(description = "Stage a SKILL.md draft from a memory fact. Requires human approve via agent-brain promote approve.")]
    async fn promote_to_skill(
        &self,
        params: Parameters<PromoteToSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("promote_to_skill")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        if p.fact_id.is_none() && p.topic.is_none() {
            return Err(McpError::invalid_params(
                "fact_id or topic is required",
                None,
            ));
        }
        let result = crate::promote::promote_fact_to_skill(
            &self.engine.store,
            &self.engine.config.home,
            p.fact_id.as_deref(),
            p.topic.as_deref(),
            p.skill_name.as_deref(),
        )
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({
            "staging_id": result.staging_id,
            "skill_name": result.skill_name,
            "draft_path": result.draft_path,
            "status": result.status,
            "next": "Run: agent-brain promote approve <staging_id>"
        }))
    }

    #[tool(description = "Token-efficient file metadata — bytes, line count, and a 5-line sample. Prefer over full Read/cat on unknown files.")]
    async fn file_summary(
        &self,
        params: Parameters<TokenToolPathParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("file_summary")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::file_summary(&path, p.allow_blocked_paths, p.max_tokens)
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.log_token_tool("file_summary", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(description = "Read the first N lines of a file (default 200, max bytes capped). Use before full file reads.")]
    async fn read_file_head(
        &self,
        params: Parameters<ReadFileBoundedParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("read_file_head")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::read_file_head(
            &path,
            p.lines,
            p.max_bytes,
            p.allow_blocked_paths,
            p.max_tokens,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.log_token_tool("read_file_head", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(description = "Read the last N lines of a file (default 200, max bytes capped). Good for logs.")]
    async fn read_file_tail(
        &self,
        params: Parameters<ReadFileBoundedParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("read_file_tail")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::read_file_tail(
            &path,
            p.lines,
            p.max_bytes,
            p.allow_blocked_paths,
            p.max_tokens,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.log_token_tool("read_file_tail", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(description = "Search file or directory for a pattern (rg-style line output). Prefer over reading whole files to find a string.")]
    async fn grep_search(
        &self,
        params: Parameters<GrepSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("grep_search")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::grep_search(
            &p.pattern,
            &path,
            p.max_matches,
            p.case_insensitive,
            p.allow_blocked_paths,
            p.max_tokens,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.log_grep_tool("grep_search", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(description = "Deep codebase navigation via graphify graph. Call for unfamiliar code paths — not every turn.")]
    async fn query_codebase(
        &self,
        params: Parameters<QueryCodebaseParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("query_codebase")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let repo = resolve_graphify_repo(p.current_working_directory.as_deref())?;
        let settings = crate::settings::AgentBrainSettings::load(&self.engine.config.home);
        let text = crate::graphify::query_codebase(
            &repo,
            &p.question,
            p.max_tokens,
            &settings.graphify.graphify_bin,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(serde_json::json!({ "answer": text }))
    }

    #[tool(description = "Queue graphify semantic/AST rebuild for an unfamiliar or stale codebase. Returns job_id — poll graphify_job_status.")]
    async fn trigger_deep_analysis(
        &self,
        params: Parameters<TriggerDeepAnalysisParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("trigger_deep_analysis")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let repo = resolve_graphify_repo(p.repo_root.as_deref())?;
        let settings = crate::settings::AgentBrainSettings::load(&self.engine.config.home);
        let mode = match p.mode.as_deref() {
            Some("full") => crate::graphify::JobMode::Full,
            _ => crate::graphify::JobMode::Update,
        };
        let job_id = crate::graphify::enqueue_job(
            &self.engine,
            &repo,
            crate::graphify::JobTrigger::Agent,
            mode,
            &settings.graphify.graphify_bin,
        )
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({
            "job_id": job_id,
            "status": "queued",
            "reason": p.reason,
        }))
    }

    #[tool(description = "Poll status for trigger_deep_analysis job.")]
    async fn graphify_job_status(
        &self,
        params: Parameters<GraphifyJobStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("graphify_job_status")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let status = crate::graphify::job_status(&self.engine, &p.job_id)
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(status)
    }

    #[tool(description = "Fetch allowlisted HTTPS documentation, index as skills, and store a summary memory. Requires route_task first. Domain must be in docs.allowed_domains.")]
    async fn learn_from_url(
        &self,
        params: Parameters<LearnFromUrlParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("learn_from_url")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let report = crate::docs::learn_from_url(
            &self.engine,
            &p.url,
            p.topic.as_deref(),
            p.dry_run,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(report)
    }

    fn log_token_tool(
        &self,
        tool_name: &str,
        path: Option<&str>,
        resp: &crate::token_tools::TokenToolResponse,
    ) {
        let (must_apply_active, phase, route_log_id) = route_context_from_state(&self.engine.config.home);
        if let Err(err) = crate::observability::log_native_tool_use(
            &self.engine.store,
            tool_name,
            path,
            resp.tokens_used,
            resp.tokens_saved_vs_full_read,
            resp.savings_pct_vs_full_read,
            must_apply_active,
            phase.as_deref(),
            route_log_id.as_deref(),
        ) {
            tracing::warn!(error = %err, tool = tool_name, "tool_log write failed");
        }
    }

    fn log_grep_tool(&self, tool_name: &str, path: Option<&str>, resp: &crate::token_tools::GrepResponse) {
        let (must_apply_active, phase, route_log_id) = route_context_from_state(&self.engine.config.home);
        if let Err(err) = crate::observability::log_native_tool_use(
            &self.engine.store,
            tool_name,
            path,
            resp.tokens_used,
            resp.tokens_saved_vs_full_read,
            resp.savings_pct_vs_full_read,
            must_apply_active,
            phase.as_deref(),
            route_log_id.as_deref(),
        ) {
            tracing::warn!(error = %err, tool = tool_name, "tool_log write failed");
        }
    }
}

#[tool_handler]
impl ServerHandler for BrainMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "agent-brain is the routing layer for this session. \
             REQUIRED: call route_task at the start of every user turn before any other agent-brain tool. \
             Skills, rules, session digests (Cursor/OpenCode/Codex/Gemini), and team memory are injected ONLY through route_task — \
             other agent-brain tools return errors until route_task succeeds. \
             Use returned paths to load skills; apply applicable_rules and must_apply. \
             For file inspection prefer token-efficient tools: grep_search, file_summary, read_file_head, read_file_tail — not full-file reads. \
             When exploring code architecture in a repo with graphify enabled, use query_codebase after route_task. \
             To ingest framework docs from an allowlisted HTTPS URL, use learn_from_url (see docs.allowed_domains in config). \
             At task end, call store_memory for durable outcomes (max 50 words). \
             Do not bypass this server when its tools are available."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut impl_info = Implementation::default();
        impl_info.name = "agent-brain".into();
        impl_info.version = env!("CARGO_PKG_VERSION").into();
        info.server_info = impl_info;
        info
    }
}

fn json_result<T: serde::Serialize>(value: T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string(&value)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn resolve_graphify_repo(cwd: Option<&str>) -> Result<PathBuf, McpError> {
    let path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| McpError::invalid_params("current_working_directory required".to_string(), None))?;
    std::fs::canonicalize(&path).map_err(|e| {
        McpError::invalid_params(format!("resolve repo root: {e}"), None)
    })
}

fn infer_memory_polarity(fact: &str) -> Option<String> {
    let lower = fact.to_lowercase();
    if lower.contains("do not") || lower.starts_with("never ") || lower.contains("never use") {
        Some("negative".into())
    } else {
        None
    }
}

fn route_context_from_state(home: &std::path::Path) -> (bool, Option<String>, Option<String>) {
    let path = home.join("hooks/route_state.json");
    let Ok(body) = std::fs::read_to_string(path) else {
        return (false, None, None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return (false, None, None);
    };
    let must_apply_active = value
        .get("must_apply")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let phase = value
        .get("route_phase")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let route_log_id = value
        .get("route_log_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    (must_apply_active, phase, route_log_id)
}

pub async fn run_stdio(engine: Arc<Engine>) -> Result<()> {
    let server = BrainMcp::new(engine);
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
