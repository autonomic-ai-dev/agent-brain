use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::sleep;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::db::store::{looks_like_secret, word_count};
use crate::db::write_queue::{store_memory_payload, WriteOp};
use crate::engine::Engine;
use crate::gc;
use crate::mcp::route_gate::McpRouteGate;
use crate::skill_registry::{read_skill_file, search_terms, MAX_SKILL_RESULTS};
use crate::types::{deserialize_route_limits, ItemType, RouteLimits};

#[derive(Clone)]
pub struct BrainMcp {
    engine: Arc<Engine>,
    #[allow(dead_code)]
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

    async fn describe_upstream_tool(
        &self,
        server: &crate::settings::UpstreamServerConfig,
        tool_name: &str,
    ) -> Result<CallToolResult, McpError> {
        let schema = crate::upstream::get_upstream_tool_schema(server, tool_name)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        match schema {
            Some(s) => json_result(serde_json::json!({
                "server": server.name,
                "tool": tool_name,
                "schema": s,
            })),
            None => Err(McpError::invalid_params(
                format!("tool '{}' not found on server '{}'", tool_name, server.name),
                None,
            )),
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
    #[serde(
        default = "default_route_limits",
        deserialize_with = "deserialize_route_limits"
    )]
    #[schemars(default = "default_route_limits")]
    limits: RouteLimits,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    task_kind: Option<String>,
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
    /// Unix timestamp in milliseconds when this fact becomes active.
    #[serde(default)]
    valid_from: Option<i64>,
    /// Unix timestamp in milliseconds when this fact expires (archived by temporal prune).
    #[serde(default)]
    invalid_at: Option<i64>,
}

fn default_scope() -> String {
    "project".into()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListMemoryParams {
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SkillSearchParams {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SkillGetParams {
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpSearchParams {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetMcpParams {
    server: String,
    tool: String,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecordTurnTelemetryParams {
    provider: String,
    model: String,
    prompt_tokens: u64,
    completion_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    latency_ms: u64,
    #[serde(default)]
    task_phase: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetCacheSavingsParams {
    #[serde(default = "default_telemetry_lookback")]
    lookback_days: u32,
}

fn default_telemetry_lookback() -> u32 {
    30
}

#[derive(Debug, Deserialize, JsonSchema)]
struct VerifyRunParams {
    #[serde(default)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecordVerifyOutcomeParams {
    exit_code: i32,
    cmd: String,
    #[serde(default)]
    cause_tag: Option<String>,
    #[serde(default)]
    route_log_id: Option<String>,
    #[serde(default)]
    config_path: Option<String>,
    #[serde(default)]
    scenarios_dir: Option<String>,
    /// Working directory used to discover project-root `.verify/`.
    #[serde(default)]
    cwd: Option<String>,
    /// cli|hook|mcp|playbook — defaults to mcp (playbook when explicitly set).
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StoreTrajectoryParams {
    workflow_id: String,
    node_id: String,
    /// One of: success, failure, escalated, skipped
    outcome: String,
    #[serde(default)]
    route_log_id: Option<String>,
    #[serde(default)]
    task_kind: Option<String>,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecallProceduresParams {
    query: String,
    #[serde(default)]
    task_kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GcParams {
    #[serde(default = "default_min_confidence")]
    min_confidence: f64,
    #[serde(default = "default_max_age_days")]
    max_age_days: u64,
}

fn default_min_confidence() -> f64 {
    0.3
}

fn default_max_age_days() -> u64 {
    90
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
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    arguments: serde_json::Value,
    #[serde(default = "default_upstream_tokens")]
    max_tokens: usize,
    #[serde(default)]
    describe_tool: bool,
    #[serde(default)]
    batch: Option<Vec<crate::upstream::UpstreamBatchItem>>,
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
    #[serde(alias = "query")]
    pattern: String,
    #[serde(default = "default_grep_path")]
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

fn default_grep_path() -> String {
    ".".to_string()
}

fn default_grep_matches() -> usize {
    crate::token_tools::DEFAULT_GREP_MAX_MATCHES
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListDirectoryParams {
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default = "default_max_directory_entries")]
    max_entries: usize,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    allow_blocked_paths: bool,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

fn default_max_directory_entries() -> usize {
    100
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindFilesParams {
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    extension: Option<String>,
    #[serde(default = "default_find_depth")]
    max_depth: usize,
    #[serde(default = "default_find_matches")]
    max_matches: usize,
    #[serde(default)]
    allow_blocked_paths: bool,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

fn default_find_depth() -> usize {
    10
}

fn default_find_matches() -> usize {
    100
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadFileParams {
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default = "default_start_line")]
    start_line: usize,
    #[serde(default = "default_end_line")]
    end_line: usize,
    #[serde(default = "default_read_bytes")]
    max_bytes: usize,
    #[serde(default)]
    allow_blocked_paths: bool,
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
}

fn default_start_line() -> usize {
    1
}

fn default_end_line() -> usize {
    200
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
struct GetAutonomicCatalogParams {
    #[serde(default)]
    organ: Option<String>,
    #[serde(default)]
    verbose: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
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
#[allow(dead_code)]
struct TriggerDeepAnalysisParams {
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
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

#[derive(Debug, Deserialize, JsonSchema)]
struct GetContextForNodeParams {
    node_kind: String,
    node_name: String,
    node_description: String,
    workflow_name: String,
    task_description: String,
    #[serde(default = "default_node_tokens")]
    max_tokens: usize,
}

fn default_node_tokens() -> usize {
    300
}

// ── KG Phase 2: symbol navigation params ────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct SearchSymbolsParams {
    query: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default = "default_symbol_limit")]
    limit: usize,
    #[serde(default)]
    current_working_directory: Option<String>,
}

fn default_symbol_limit() -> usize {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct GetSymbolDefinitionParams {
    name: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    current_working_directory: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct GetFileOutlineParams {
    path: String,
    #[serde(default)]
    current_working_directory: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct FindCallersParams {
    name: String,
    #[serde(default)]
    current_working_directory: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct GetLocalGraphParams {
    name: String,
    #[serde(default = "default_graph_depth")]
    depth: usize,
    #[serde(default)]
    current_working_directory: Option<String>,
}

fn default_graph_depth() -> usize {
    2
}

// ── Phase 6: scratchpad params ────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct WriteScratchpadParams {
    content: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default = "default_scratchpad_type")]
    context_type: String,
    #[serde(default)]
    tags: Option<String>,
    #[serde(default)]
    current_working_directory: Option<String>,
}

fn default_scratchpad_type() -> String {
    "note".into()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadScratchpadParams {
    #[serde(default = "default_scratchpad_limit")]
    limit: usize,
    #[serde(default)]
    context_type: Option<String>,
    #[serde(default)]
    current_working_directory: Option<String>,
}

fn default_scratchpad_limit() -> usize {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct RecentSymbolsParams {
    #[serde(default = "default_symbol_limit")]
    limit: usize,
    #[serde(default)]
    current_working_directory: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GitLogParams {
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default)]
    range: Option<String>,
    #[serde(default = "default_git_log_count")]
    max_count: usize,
}

fn default_git_log_count() -> usize {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GitDiffParams {
    #[serde(default)]
    current_working_directory: Option<String>,
    range: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GitTagsParams {
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default = "default_git_tags_count")]
    max_count: usize,
}

fn default_git_tags_count() -> usize {
    20
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GitCompareParams {
    #[serde(default)]
    current_working_directory: Option<String>,
    base: String,
    head: String,
}

#[tool_router]
impl BrainMcp {
    #[tool(
        description = "REQUIRED every turn before planning or edits. Returns ranked agents, skills, rules, and memory under a token budget. Pass user_message, current_working_directory, and open_files. Session digests from Cursor/OpenCode/Codex/Gemini/Antigravity and team memory are only injected here."
    )]
    async fn route_task(
        &self,
        params: Parameters<RouteTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let resp = self
            .engine
            .route_task(
                &p.user_message,
                cwd.as_deref(),
                &p.open_files,
                p.max_tokens,
                p.limits.normalize(),
                p.phase.as_deref(),
                p.task_kind.as_deref(),
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        self.route_gate.record_route(&p.user_message);
        json_result(resp)
    }

    #[tool(
        description = "Returns the capability catalog and delegation rules for agent-brain and agent-spine. Call during agent initialization or before complex tasks to discover available tools."
    )]
    async fn get_autonomic_catalog(
        &self,
        params: Parameters<GetAutonomicCatalogParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let catalog = build_autonomic_catalog(p.organ.as_deref(), p.verbose);
        json_result(catalog)
    }

    #[tool(
        description = "Lower-level flat context retrieval. Requires route_task first in this turn."
    )]
    async fn get_context(
        &self,
        params: Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("get_context")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let types: Vec<ItemType> = p
            .include_types
            .iter()
            .filter_map(|s| ItemType::parse(s))
            .collect();
        let resp = self
            .engine
            .get_context(&p.task_description, cwd.as_deref(), p.max_tokens, &types)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(resp)
    }

    #[tool(
        description = "Fetch specialized rules/skills/agents for a workflow node. Call from agent-spine before executing a node to hydrate subagent prompts with brain context."
    )]
    async fn get_context_for_node(
        &self,
        params: Parameters<GetContextForNodeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("get_context_for_node")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let ctx_params = crate::node_context::NodeContextParams {
            node_kind: p.node_kind,
            node_name: p.node_name,
            node_description: p.node_description,
            workflow_name: p.workflow_name,
            task_description: p.task_description,
        };
        let resp =
            crate::node_context::get_context_for_node(&self.engine, &ctx_params, p.max_tokens)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(resp)
    }

    #[tool(
        description = "REQUIRED at task end for durable decisions. Max 50 words. No secrets. Optional valid_from/invalid_at (unix ms). Requires route_task first in this turn."
    )]
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
        if let (Some(from), Some(until)) = (p.valid_from, p.invalid_at) {
            if until <= from {
                return Err(McpError::invalid_params(
                    "invalid_at must be after valid_from",
                    None,
                ));
            }
        }

        let scope_key = std::env::current_dir().ok().and_then(|cwd| {
            let identity = crate::project_key::resolve_project_key(&cwd).ok()?;
            let domain = self
                .engine
                .store
                .domain_for_project_key(&identity.project_key)
                .unwrap_or_else(|_| "unclassified".to_string());
            let _ = self.engine.store.upsert_project_root(
                &identity.project_key,
                &identity.repo_root,
                &domain,
            );
            if p.scope == "project" {
                Some(identity.project_key)
            } else {
                None
            }
        });

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
                    confidence: if p.confidence == 0.0 {
                        0.9
                    } else {
                        p.confidence
                    },
                    polarity,
                    apply_when: p.apply_when,
                    valid_from: p.valid_from,
                    invalid_at: p.invalid_at,
                },
            })
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

        let value = rx
            .recv()
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(value)
    }

    #[tool(
        description = "Record an already-known exit without wrapping the command (agent-asserted; not trusted for learning). Prefer `agent-brain verify -- <cmd>` or `--source orchestrator`. Hashes project-root `.verify/` when present. Does not require route_task."
    )]
    async fn record_verify_outcome(
        &self,
        params: Parameters<RecordVerifyOutcomeParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let source_raw = p.source.as_deref().unwrap_or("mcp");
        let source = crate::verify::VerifySource::parse(source_raw)
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let config_path = p.config_path.as_ref().map(PathBuf::from);
        let scenarios = p.scenarios_dir.as_ref().map(PathBuf::from);
        let cwd = p.cwd.as_ref().map(PathBuf::from);
        let report = crate::verify::record_from_result(
            &self.engine.store,
            p.exit_code,
            &p.cmd,
            p.route_log_id.as_deref(),
            p.cause_tag.as_deref(),
            config_path.as_deref(),
            scenarios.as_deref(),
            cwd.as_deref(),
            source,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(serde_json::json!({ "verify": report }))
    }

    #[tool(
        description = "Run the project's [verify.default] step (allowlisted and time-limited) and record it as trusted cli evidence, so the stop gate counts it. Returns the report and, on failure, the output tail. Omit project to use the current repo."
    )]
    async fn verify_run(
        &self,
        params: Parameters<VerifyRunParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let project = params
            .0
            .project
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let engine = self.engine.clone();
        let value = tokio::task::spawn_blocking(move || {
            crate::verify::run_project_verify(&engine.store, &project)
        })
        .await
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(value)
    }

    #[tool(
        description = "Record orchestrator workflow node outcome for learning loops. Links optional route_log_id from route_task. Does not require route_task in this turn."
    )]
    async fn store_trajectory(
        &self,
        params: Parameters<StoreTrajectoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let report = crate::trajectory::store_trajectory(
            &self.engine.store,
            &p.workflow_id,
            &p.node_id,
            &p.outcome,
            p.route_log_id.as_deref(),
            p.task_kind.as_deref(),
            p.notes.as_deref(),
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(serde_json::json!({ "trajectory": report }))
    }

    #[tool(
        description = "Recall proven procedures and workflow steps based on task intent and historical trajectories."
    )]
    async fn recall_procedures(
        &self,
        params: Parameters<RecallProceduresParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let limit = p.limit.unwrap_or(5);
        let items = crate::retrieval::recall_procedures(
            &self.engine.store,
            &p.query,
            p.task_kind.as_deref(),
            limit,
        )
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({ "procedures": items }))
    }

    #[tool(description = "Run garbage collection: dedup, prune, vacuum")]
    async fn brain_gc(&self, params: Parameters<GcParams>) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let stats = gc::run_gc(&self.engine.store, p.min_confidence, p.max_age_days)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(stats)
    }

    #[tool(description = "Record LLM turn telemetry, calculate prompt/cache costs and savings, .")]
    async fn record_turn_telemetry(
        &self,
        params: Parameters<RecordTurnTelemetryParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let req = crate::telemetry::TurnTelemetryRequest {
            session_id: p.session_id,
            provider: p.provider,
            model: p.model,
            prompt_tokens: p.prompt_tokens,
            completion_tokens: p.completion_tokens,
            cache_read_tokens: p.cache_read_tokens,
            cache_write_tokens: p.cache_write_tokens,
            latency_ms: p.latency_ms,
            task_phase: p.task_phase.clone(),
        };
        let outcome = crate::telemetry::process_turn_telemetry(req);
        self.engine
            .store
            .insert_turn_telemetry(&outcome, p.task_phase.as_deref())
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(outcome)
    }

    #[tool(
        description = "Get aggregated cache savings and token attribution report across recent agent turns."
    )]
    async fn get_cache_savings_summary(
        &self,
        params: Parameters<GetCacheSavingsParams>,
    ) -> Result<CallToolResult, McpError> {
        let _req = self.engine.mcp_activity.begin_request();
        let summary = self
            .engine
            .store
            .query_turn_telemetry_summary(params.0.lookback_days)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(summary)
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
        let filename = params
            .0
            .filename
            .unwrap_or_else(|| format!("export-{}.json", chrono::Utc::now().timestamp()));
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
        let explain =
            crate::observability::explain_last(&self.engine.store, params.0.log_id.as_deref())
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({ "context": explain }))
    }

    #[tool(
        description = "Search the skill registry by keywords; every word must match. Returns skill names and paths. Use skill_get to load one."
    )]
    async fn skill_search(
        &self,
        params: Parameters<SkillSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("skill_search")?;
        let _req = self.engine.mcp_activity.begin_request();
        let terms = search_terms(&params.0.query);
        if terms.is_empty() {
            return Err(McpError::invalid_params(
                "query must contain at least one word",
                None,
            ));
        }
        let limit = params.0.limit.min(MAX_SKILL_RESULTS);
        let rows = self
            .engine
            .store
            .find_skill_items(&terms, limit)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let skills: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|row| serde_json::json!({ "name": row.name, "path": row.path }))
            .collect();
        json_result(serde_json::json!({ "skills": skills }))
    }

    #[tool(description = "Fetch the full SKILL.md instructions for a skill by its registry name.")]
    async fn skill_get(
        &self,
        params: Parameters<SkillGetParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("skill_get")?;
        let _req = self.engine.mcp_activity.begin_request();
        let source = self
            .engine
            .store
            .skill_source_path(&params.0.name)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let Some(source) = source else {
            return Err(McpError::invalid_params(
                format!("skill not found: {}", params.0.name),
                None,
            ));
        };
        let content = read_skill_file(Path::new(&source)).map_err(|_| {
            McpError::internal_error("skill file is unavailable; rebuild the index", None)
        })?;
        json_result(serde_json::json!({ "name": params.0.name, "content": content }))
    }

    #[tool(
        description = "Search configured upstream MCP tools by keywords; every word must match the server name, tool name, or description. Returns server and tool names. Use get_mcp to load a tool's schema, then route_to_mcp to call it."
    )]
    async fn mcp_search(
        &self,
        params: Parameters<McpSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("mcp_search")?;
        let _req = self.engine.mcp_activity.begin_request();
        let terms = search_terms(&params.0.query);
        if terms.is_empty() {
            return Err(McpError::invalid_params(
                "query must contain at least one word",
                None,
            ));
        }
        let limit = params.0.limit.min(MAX_SKILL_RESULTS);
        let tools: Vec<serde_json::Value> = self
            .engine
            .store
            .list_upstream_tools()
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?
            .into_iter()
            .filter(|tool| {
                let haystack = format!("{} {} {}", tool.server, tool.name, tool.description)
                    .to_ascii_lowercase();
                terms.iter().all(|term| haystack.contains(term))
            })
            .take(limit)
            .map(|tool| {
                serde_json::json!({
                    "server": tool.server,
                    "tool": tool.name,
                    "description": tool.description,
                })
            })
            .collect();
        json_result(serde_json::json!({ "tools": tools }))
    }

    #[tool(
        description = "Return the input schema for one upstream MCP tool found with mcp_search. Run it with route_to_mcp."
    )]
    async fn get_mcp(&self, params: Parameters<GetMcpParams>) -> Result<CallToolResult, McpError> {
        self.require_route("get_mcp")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let settings = crate::settings::AgentBrainSettings::load(&self.engine.config.home);
        let server =
            crate::upstream::find_server(&settings.upstream_mcp, &p.server).ok_or_else(|| {
                McpError::invalid_params(
                    format!("unknown or disabled upstream server: {}", p.server),
                    None,
                )
            })?;
        self.describe_upstream_tool(server, &p.tool).await
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

    #[tool(
        description = "Call a configured upstream MCP tool, batch of tools, or inspect a tool schema. Response is semantically truncated to max_tokens. Agent must call explicitly; router never auto-executes upstream tools."
    )]
    async fn route_to_mcp(
        &self,
        params: Parameters<RouteToMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("route_to_mcp")?;
        let started = std::time::Instant::now();
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let settings = crate::settings::AgentBrainSettings::load(&self.engine.config.home);
        let server =
            crate::upstream::find_server(&settings.upstream_mcp, &p.server).ok_or_else(|| {
                McpError::invalid_params(
                    format!("unknown or disabled upstream server: {}", p.server),
                    None,
                )
            })?;

        if p.describe_tool {
            let tool_name = p.tool.as_deref().ok_or_else(|| {
                McpError::invalid_params("tool name required when describe_tool is true", None)
            })?;
            return self.describe_upstream_tool(server, tool_name).await;
        }

        if let Some(batch) = p.batch {
            let count = batch.len();
            let outcomes = crate::upstream::call_upstream_tools_batch(server, batch)
                .await
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            let is_any_error = outcomes.iter().any(|o| o.is_error);
            let log_id = uuid::Uuid::new_v4().to_string();
            let latency_ms = started.elapsed().as_millis() as u64;

            let serialized = serde_json::to_string(&outcomes).unwrap_or_default();
            let structured_val = serde_json::to_value(&outcomes).ok();
            let truncated = crate::upstream::truncate_upstream_result(
                &serialized,
                structured_val.as_ref(),
                p.max_tokens,
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

            if let Err(err) = crate::observability::log_upstream_call(
                &self.engine.store,
                &log_id,
                &server.name,
                &format!("batch({})", count),
                truncated.tokens_used,
                truncated.truncated,
                is_any_error,
                latency_ms,
            ) {
                tracing::warn!(error = %err, "upstream retrieval log write failed");
            }

            return json_result(serde_json::json!({
                "log_id": log_id,
                "server": server.name,
                "batch_count": count,
                "is_error": is_any_error,
                "truncated": truncated.truncated,
                "tokens_used": truncated.tokens_used,
                "tokens_budget": truncated.tokens_budget,
                "content": truncated.content,
            }));
        }

        let tool_name = p
            .tool
            .ok_or_else(|| McpError::invalid_params("tool name or batch array required", None))?;
        let arguments = if p.arguments.is_null() {
            serde_json::json!({})
        } else {
            p.arguments
        };
        let result = crate::upstream::call_upstream_tool(server, &tool_name, arguments)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let is_error = result.is_error.unwrap_or(false);
        let (raw, structured) = crate::upstream::call_tool_result_to_text(&result);
        let truncated =
            crate::upstream::truncate_upstream_result(&raw, structured.as_ref(), p.max_tokens)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let log_id = uuid::Uuid::new_v4().to_string();
        let latency_ms = started.elapsed().as_millis() as u64;
        if let Err(err) = crate::observability::log_upstream_call(
            &self.engine.store,
            &log_id,
            &server.name,
            &tool_name,
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
            "tool": tool_name,
            "is_error": is_error,
            "truncated": truncated.truncated,
            "tokens_used": truncated.tokens_used,
            "tokens_budget": truncated.tokens_budget,
            "content": truncated.content,
        }))
    }

    #[tool(
        description = "Stage a SKILL.md draft from a memory fact. Requires human approve via agent-brain promote approve."
    )]
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

    #[tool(
        description = "Token-efficient file metadata — bytes, line count, and a 5-line sample. Prefer over full Read/cat on unknown files."
    )]
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
        self.engine.record_file_access("file_summary", &p.path);
        self.log_token_tool("file_summary", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(
        description = "Read the first N lines of a file (default 200, max bytes capped). Use before full file reads."
    )]
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
        self.engine.record_file_access("read_file_head", &p.path);
        self.log_token_tool("read_file_head", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(
        description = "Read the last N lines of a file (default 200, max bytes capped). Good for logs."
    )]
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
        self.engine.record_file_access("read_file_tail", &p.path);
        self.log_token_tool("read_file_tail", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(
        description = "Search file or directory for a pattern (rg-style line output). Prefer over reading whole files to find a string."
    )]
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
        self.engine.record_file_access("grep_search", &p.path);
        self.log_grep_tool("grep_search", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(
        description = "Read a specific line range from a file (1-indexed start_line to end_line). Token-bounded alternative to host cat/sed/awk."
    )]
    async fn read_file(
        &self,
        params: Parameters<ReadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("read_file")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::read_file(
            &path,
            p.start_line,
            p.end_line,
            p.max_bytes,
            p.allow_blocked_paths,
            p.max_tokens,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.engine.record_file_access("read_file", &p.path);
        self.log_token_tool("read_file", Some(&p.path), &resp);
        json_result(resp)
    }

    #[tool(
        description = "List files and subdirectories in a directory with types, sizes, and timestamps. Token-bounded alternative to host ls."
    )]
    async fn list_directory(
        &self,
        params: Parameters<ListDirectoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("list_directory")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::list_directory(
            &path,
            p.max_entries,
            p.include_hidden,
            p.allow_blocked_paths,
            p.max_tokens,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.engine.record_file_access("list_directory", &p.path);
        json_result(resp)
    }

    #[tool(
        description = "Recursively find files matching name pattern, glob, or extension. Token-bounded alternative to host find."
    )]
    async fn find_files(
        &self,
        params: Parameters<FindFilesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("find_files")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_ref().map(PathBuf::from);
        let path = crate::token_tools::resolve_tool_path(&p.path, cwd.as_deref())
            .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        let resp = crate::token_tools::find_files(
            &path,
            p.pattern.as_deref(),
            p.extension.as_deref(),
            p.max_depth,
            p.max_matches,
            p.allow_blocked_paths,
            p.max_tokens,
        )
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        self.engine.record_file_access("find_files", &p.path);
        json_result(resp)
    }

    #[tool(
        description = "Deep codebase navigation via graphify graph. Call for unfamiliar code paths — not every turn."
    )]
    async fn query_codebase(
        &self,
        #[allow(unused_variables)] params: Parameters<QueryCodebaseParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("query_codebase"));
        }

        #[cfg(feature = "extras")]
        {
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
    }

    #[tool(
        description = "Queue graphify semantic/AST rebuild for an unfamiliar or stale codebase. Returns job_id — poll graphify_job_status."
    )]
    async fn trigger_deep_analysis(
        &self,
        #[allow(unused_variables)] params: Parameters<TriggerDeepAnalysisParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("trigger_deep_analysis"));
        }

        #[cfg(feature = "extras")]
        {
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
    }

    #[tool(description = "Poll status for trigger_deep_analysis job.")]
    async fn graphify_job_status(
        &self,
        #[allow(unused_variables)] params: Parameters<GraphifyJobStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("graphify_job_status"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("graphify_job_status")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let status = crate::graphify::job_status(&self.engine, &p.job_id)
                .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
            json_result(status)
        }
    }

    #[tool(
        description = "Fetch allowlisted HTTPS documentation, index as skills, and store a summary memory. Requires route_task first. Domain must be in docs.allowed_domains."
    )]
    async fn learn_from_url(
        &self,
        params: Parameters<LearnFromUrlParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("learn_from_url")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let report =
            crate::docs::learn_from_url(&self.engine, &p.url, p.topic.as_deref(), p.dry_run)
                .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;
        json_result(report)
    }

    fn log_token_tool(
        &self,
        tool_name: &str,
        path: Option<&str>,
        resp: &crate::token_tools::TokenToolResponse,
    ) {
        let (must_apply_active, phase, route_log_id) =
            route_context_from_state(&self.engine.config.home);
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

    fn log_grep_tool(
        &self,
        tool_name: &str,
        path: Option<&str>,
        resp: &crate::token_tools::GrepResponse,
    ) {
        let (must_apply_active, phase, route_log_id) =
            route_context_from_state(&self.engine.config.home);
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

    // ── KG Phase 2: symbol navigation tools ────────────────

    #[tool(
        description = "Search code symbols by name across the current repo. Optionally filter by language or file. Results are ranked by label similarity."
    )]
    async fn search_symbols(
        &self,
        #[allow(unused_variables)] params: Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("search_symbols"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("search_symbols")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
            let repo_root = path.display().to_string();
            let results = self
                .engine
                .store
                .search_symbols(
                    &repo_root,
                    &p.query,
                    p.language.as_deref(),
                    p.file.as_deref(),
                    p.limit,
                )
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            json_result(results)
        }
    }

    #[tool(
        description = "Get the full definition (source + docstring) of a named symbol. Optionally scope to a file."
    )]
    async fn get_symbol_definition(
        &self,
        #[allow(unused_variables)] params: Parameters<GetSymbolDefinitionParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("get_symbol_definition"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("get_symbol_definition")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
            let repo_root = path.display().to_string();
            let result = self
                .engine
                .store
                .get_symbol_definition(&repo_root, &p.name, p.file.as_deref())
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            json_result(result)
        }
    }

    #[tool(
        description = "Get all symbols defined in a file, sorted by start line. Returns label, kind, lines, and source for each symbol."
    )]
    async fn get_file_outline(
        &self,
        #[allow(unused_variables)] params: Parameters<GetFileOutlineParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("get_file_outline"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("get_file_outline")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
            let repo_root = path.display().to_string();
            let results = self
                .engine
                .store
                .get_file_outline(&repo_root, &p.path)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            json_result(results)
        }
    }

    #[tool(
        description = "Find all callers of a named symbol. Returns caller symbols with source locations."
    )]
    async fn find_callers(
        &self,
        #[allow(unused_variables)] params: Parameters<FindCallersParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("find_callers"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("find_callers")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
            let repo_root = path.display().to_string();
            let results = self
                .engine
                .store
                .find_callers(&repo_root, &p.name)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            json_result(results)
        }
    }

    #[tool(
        description = "Explore the local dependency graph around a symbol via BFS over code_graph_edges. Depth defaults to 2 hops (max 5)."
    )]
    async fn get_local_graph(
        &self,
        #[allow(unused_variables)] params: Parameters<GetLocalGraphParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("get_local_graph"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("get_local_graph")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
            let repo_root = path.display().to_string();
            let results = self
                .engine
                .store
                .get_local_graph(&repo_root, &p.name, p.depth)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            json_result(results)
        }
    }

    // ── Phase 6: scratchpad tools ───────────────────────────

    #[tool(
        description = "Write a note to the cross-agent scratchpad. Other agents will see this in their next route_task briefing. Use context_type='decision' for binding decisions, 'insight' for observations, 'blocker' for stuck items."
    )]
    async fn write_scratchpad(
        &self,
        params: Parameters<WriteScratchpadParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("write_scratchpad")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
        let repo_root = path.display().to_string();
        let id = self
            .engine
            .store
            .write_scratchpad(
                &repo_root,
                p.agent_id.as_deref(),
                &p.content,
                &p.context_type,
                p.tags.as_deref(),
            )
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(serde_json::json!({"id": id}))
    }

    #[tool(
        description = "Read recent scratchpad entries for this repo. Optionally filter by context_type (note/decision/insight/blocker). Ordered newest-first."
    )]
    async fn read_scratchpad(
        &self,
        params: Parameters<ReadScratchpadParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("read_scratchpad")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
        let repo_root = path.display().to_string();
        let entries = self
            .engine
            .store
            .read_scratchpad(&repo_root, p.limit, p.context_type.as_deref())
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        json_result(entries)
    }

    #[tool(
        description = "List recently indexed AST symbols (code_graph_nodes with ast_symbol), newest first. Useful to see what the indexer has discovered recently."
    )]
    async fn recent_symbols(
        &self,
        #[allow(unused_variables)] params: Parameters<RecentSymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(not(feature = "extras"))]
        {
            return Err(missing_extras_mcp("recent_symbols"));
        }

        #[cfg(feature = "extras")]
        {
            self.require_route("recent_symbols")?;
            let _req = self.engine.mcp_activity.begin_request();
            let p = params.0;
            let path = resolve_graphify_repo(p.current_working_directory.as_deref())?;
            let repo_root = path.display().to_string();
            let results = self
                .engine
                .store
                .recent_symbols(&repo_root, p.limit)
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            json_result(results)
        }
    }

    #[tool(
        description = "Show commit log with optional range (e.g. v1.0..v2.0). Returns structured entries with commit hash, author, date, message."
    )]
    async fn brain_git_log(
        &self,
        params: Parameters<GitLogParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("brain_git_log")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_deref().map(PathBuf::from);
        match crate::git_tools::git_log(
            cwd.as_deref().unwrap_or_else(|| std::path::Path::new(".")),
            p.range.as_deref(),
            p.max_count,
        ) {
            Ok(entries) => json_result(entries),
            Err(e) => Err(McpError::internal_error(e, None)),
        }
    }

    #[tool(
        description = "Show diff between two refs (e.g. v1.0..v2.0). Optionally filter by path. Returns diff text with file/insertion/deletion counts."
    )]
    async fn brain_git_diff(
        &self,
        params: Parameters<GitDiffParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("brain_git_diff")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_deref().map(PathBuf::from);
        match crate::git_tools::git_diff(
            cwd.as_deref().unwrap_or_else(|| std::path::Path::new(".")),
            &p.range,
            p.path.as_deref(),
        ) {
            Ok(result) => json_result(result),
            Err(e) => Err(McpError::internal_error(e, None)),
        }
    }

    #[tool(
        description = "List git tags sorted by version, with optional pattern filter (e.g. v0.30.*). Returns structured tag entries."
    )]
    async fn brain_git_tags(
        &self,
        params: Parameters<GitTagsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("brain_git_tags")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_deref().map(PathBuf::from);
        match crate::git_tools::git_tags(
            cwd.as_deref().unwrap_or_else(|| std::path::Path::new(".")),
            p.pattern.as_deref(),
            p.max_count,
        ) {
            Ok(tags) => json_result(tags),
            Err(e) => Err(McpError::internal_error(e, None)),
        }
    }

    #[tool(
        description = "Compare two refs (base..head). Returns commit list, diff stats, and ahead count. Useful for release diffing."
    )]
    async fn brain_git_compare(
        &self,
        params: Parameters<GitCompareParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_route("brain_git_compare")?;
        let _req = self.engine.mcp_activity.begin_request();
        let p = params.0;
        let cwd = p.current_working_directory.as_deref().map(PathBuf::from);
        match crate::git_tools::git_compare(
            cwd.as_deref().unwrap_or_else(|| std::path::Path::new(".")),
            &p.base,
            &p.head,
        ) {
            Ok(result) => json_result(result),
            Err(e) => Err(McpError::internal_error(e, None)),
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
              For inspecting unfamiliar files, prefer token-efficient tools: grep_search, file_summary, read_file_head, read_file_tail — not full-file reads. \
              For git operations, prefer brain_git_log, brain_git_diff, brain_git_tags, brain_git_compare over raw bash. \
              When exploring code architecture in a repo with graphify enabled, use query_codebase after route_task. \
              To ingest framework docs from an allowlisted HTTPS URL, use learn_from_url (see docs.allowed_domains in config). \
              At task end, call store_memory for durable outcomes (max 50 words). \
              Native tools (read, grep, edit, write, bash, glob) are fine for operations agent-brain does not provide alternatives for."
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
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

#[cfg(not(feature = "extras"))]
fn missing_extras_mcp(tool: &str) -> McpError {
    McpError::invalid_params(
        format!(
            "tool `{tool}` requires a build with `--features extras` (graphify/code-nav harness)"
        ),
        None,
    )
}

fn resolve_graphify_repo(cwd: Option<&str>) -> Result<PathBuf, McpError> {
    let path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| {
            McpError::invalid_params("current_working_directory required".to_string(), None)
        })?;
    std::fs::canonicalize(&path)
        .map_err(|e| McpError::invalid_params(format!("resolve repo root: {e}"), None))
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

/// Serve one MCP session over an arbitrary async read/write pair (stdio or a Unix socket).
pub async fn serve_mcp_over_io<R, W>(engine: Arc<Engine>, reader: R, writer: W) -> Result<()>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let server = BrainMcp::new(engine);
    let service = server.serve((reader, writer)).await?;
    service.waiting().await?;
    Ok(())
}

/// Read the first meaningful line from stdin, answering any `server/discover`
/// probes (Antigravity / go-sdk clients probe before `initialize`) with
/// `-32601 Method not found` so they fall back to the standard handshake.
/// Returns the first real message line, or `None` on EOF.
async fn first_stdin_line(
    stdin: &mut BufReader<tokio::io::Stdin>,
    stdout: &mut tokio::io::Stdout,
) -> Result<Option<Vec<u8>>> {
    loop {
        let mut line = String::new();
        let n = stdin.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
            if val.get("method").and_then(|m| m.as_str()) == Some("server/discover") {
                let id = val.get("id").cloned().unwrap_or(serde_json::json!(1));
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": "Method not found: server/discover"
                    }
                });
                stdout.write_all(format!("{}\n", resp).as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        }
        return Ok(Some(line.into_bytes()));
    }
}

pub async fn run_stdio(engine: Arc<Engine>) -> Result<()> {
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let initial_line = match first_stdin_line(&mut stdin, &mut stdout).await? {
        Some(line) => line,
        None => return Ok(()),
    };
    let combined_reader = std::io::Cursor::new(initial_line).chain(stdin);
    serve_mcp_over_io(engine, combined_reader, stdout).await
}

/// Thin stdio proxy over the supervised brain daemon.
///
/// Connects to `socket_path` (a Unix domain socket owned by `agent-brain serve --daemon`),
/// bridges stdin/stdout to it, and reconnects if the daemon restarts. Multiple hosts can
/// run a proxy simultaneously without touching the exclusive serve lock.
pub async fn run_proxy(socket_path: &Path) -> Result<()> {
    let host_pid = unsafe { libc::getppid() };
    let mut spawned = false;
    let handshake = Arc::new(std::sync::Mutex::new(ProxyHandshake::default()));
    loop {
        if unsafe { libc::getppid() } != host_pid {
            tracing::info!("proxy: host process exited, stopping");
            return Ok(());
        }
        let stream = match UnixStream::connect(socket_path).await {
            Ok(stream) => stream,
            Err(err) => {
                if !spawned {
                    spawned = true;
                    if let Ok(current_exe) = std::env::current_exe() {
                        tracing::info!("spawning background agent-brain daemon");
                        let _ = std::process::Command::new(current_exe)
                            .arg("serve")
                            .arg("--daemon")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                    }
                }
                tracing::warn!(
                    error = %err,
                    path = %socket_path.display(),
                    "proxy: daemon not reachable, retrying"
                );
                sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        match serve_one_proxy_connection(stream, handshake.clone()).await {
            Ok(true) => return Ok(()),
            Ok(false) => tracing::debug!("proxy: daemon connection closed, reconnecting"),
            Err(err) => tracing::warn!(error = %err, "proxy: connection closed, reconnecting"),
        }
        sleep(Duration::from_millis(500)).await;
    }
}

const PROXY_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct ProxyHandshake {
    initialize: Option<String>,
    initialized: Option<String>,
}

impl ProxyHandshake {
    fn record(&mut self, line: &str) {
        let method = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| {
                value
                    .get("method")
                    .and_then(|m| m.as_str())
                    .map(str::to_owned)
            });
        match method.as_deref() {
            Some("initialize") => self.initialize = Some(line.to_string()),
            Some("notifications/initialized") => self.initialized = Some(line.to_string()),
            _ => {}
        }
    }

    fn replay(&self) -> Vec<String> {
        self.initialize
            .iter()
            .chain(self.initialized.iter())
            .cloned()
            .collect()
    }
}

async fn serve_one_proxy_connection(
    stream: UnixStream,
    handshake: Arc<std::sync::Mutex<ProxyHandshake>>,
) -> Result<bool> {
    let (rx, mut tx) = stream.into_split();
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let initial_line = match first_stdin_line(&mut stdin, &mut stdout).await? {
        Some(line) => line,
        None => return Ok(true),
    };
    let replayed = handshake.lock().unwrap().replay();
    let replay_id = replayed.first().and_then(|line| request_id(line));
    for line in &replayed {
        tx.write_all(line.as_bytes()).await?;
        tx.write_all(b"\n").await?;
    }
    let pending = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        String,
        std::time::Instant,
    >::new()));
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if stdout.write_all(line.as_bytes()).await.is_err()
                || stdout.write_all(b"\n").await.is_err()
                || stdout.flush().await.is_err()
            {
                break;
            }
        }
    });
    let from_host = {
        let pending = pending.clone();
        let handshake = handshake.clone();
        tokio::spawn(async move {
            let host = std::io::Cursor::new(initial_line).chain(stdin);
            let mut lines = BufReader::new(host).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                handshake.lock().unwrap().record(&line);
                if let Some(id) = request_id(&line) {
                    pending
                        .lock()
                        .unwrap()
                        .insert(id, std::time::Instant::now());
                }
                if tx.write_all(line.as_bytes()).await.is_err()
                    || tx.write_all(b"\n").await.is_err()
                {
                    return false;
                }
            }
            true
        })
    };
    let from_daemon = {
        let pending = pending.clone();
        let out_tx = out_tx.clone();
        let replay_id = replay_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(rx).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let id = response_id(&line);
                if id.is_some() && id == replay_id {
                    continue;
                }
                if let Some(id) = id {
                    pending.lock().unwrap().remove(&id);
                }
                if out_tx.send(line).is_err() {
                    break;
                }
            }
        })
    };
    let watchdog = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let expired: Vec<String> = {
                let mut map = pending.lock().unwrap();
                let expired: Vec<String> = map
                    .iter()
                    .filter(|(_, sent)| sent.elapsed() > PROXY_REQUEST_TIMEOUT)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in &expired {
                    map.remove(id);
                }
                expired
            };
            for id in expired {
                let _ = out_tx.send(timeout_response(&id));
            }
        }
    });

    let host_closed = tokio::select! {
        closed = from_host => closed.unwrap_or(false),
        _ = from_daemon => false,
    };
    watchdog.abort();
    writer.abort();
    Ok(host_closed)
}

fn request_id(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value.get("method")?;
    value.get("id").map(|id| id.to_string())
}

fn response_id(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("result").is_none() && value.get("error").is_none() {
        return None;
    }
    value.get("id").map(|id| id.to_string())
}

fn timeout_response(id: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": serde_json::from_str::<serde_json::Value>(id).unwrap_or(serde_json::Value::Null),
        "error": {
            "code": -32000,
            "message": format!(
                "agent-brain did not answer within {}s; the daemon may be hung. Run `agent-brain doctor`, then restart serve or this session.",
                PROXY_REQUEST_TIMEOUT.as_secs()
            )
        }
    })
    .to_string()
}

/// Supervised brain daemon: a single process owns the exclusive serve lock and `brain.db`,
/// listening on a Unix socket and serving one MCP session per connection.
///
/// Hosts connect via `agent-brain serve` (the proxy). If the daemon restarts, proxies
/// reconnect; if a host kills its proxy, nothing is lost.
pub async fn run_daemon(
    engine: Arc<Engine>,
    socket_path: &Path,
    health_addr: Option<SocketAddr>,
) -> Result<()> {
    let _ = fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    if let Some(addr) = health_addr {
        tokio::spawn(serve_health(addr));
    }
    tracing::info!(
        path = %socket_path.display(),
        "brain daemon listening on unix socket"
    );
    loop {
        let (stream, _peer) = listener.accept().await?;
        let engine = engine.clone();
        tokio::spawn(async move {
            // `UnixStream` has no `try_clone` in tokio 1.52, so round-trip through the std
            // stream (which supports `try_clone`) to get two independent `'static` handles:
            // one for reading MCP messages, one for writing responses.
            let std_stream = match stream.into_std() {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(error = %err, "daemon: into_std failed");
                    return;
                }
            };
            let std_reader = match std_stream.try_clone() {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(error = %err, "daemon: try_clone failed");
                    return;
                }
            };
            let reader = match UnixStream::from_std(std_reader) {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(error = %err, "daemon: from_std reader failed");
                    return;
                }
            };
            let writer = match UnixStream::from_std(std_stream) {
                Ok(w) => w,
                Err(err) => {
                    tracing::warn!(error = %err, "daemon: from_std writer failed");
                    return;
                }
            };
            if let Err(err) = serve_mcp_over_io(engine, reader, writer).await {
                tracing::warn!(error = %err, "daemon session ended");
            }
        });
    }
}

/// Tiny TCP health endpoint so `autonomic supervise` can liveness-check the brain daemon.
async fn serve_health(addr: SocketAddr) {
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::warn!(error = %err, addr = %addr, "health listener failed");
            return;
        }
    };
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf).await;
                    let resp =
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                    let _ = stream.write_all(resp).await;
                    let _ = stream.shutdown().await;
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, "health accept failed");
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

fn build_autonomic_catalog(organ_filter: Option<&str>, verbose: bool) -> serde_json::Value {
    let mut organs = vec![
        serde_json::json!({
            "name": "agent-spine",
            "role": "Deterministic Workflow Runner",
            "description": "Runs YAML workflow DAGs with explicit state transitions and approval gates.",
            "primary_tools": [],
            "delegation_guideline": "For multi-step repeatable tasks with approval gates, run agent-spine run --workflow <name>."
        }),
        serde_json::json!({
            "name": "agent-brain",
            "role": "Cognitive Router & Durable Memory",
            "description": "Semantic route gating, team rules, verified receipts, and durable knowledge storage.",
            "primary_tools": [
                "route_task",
                "store_memory",
                "store_trajectory",
                "record_verify_outcome",
                "verify_run",
                "list_directory",
                "find_files",
                "read_file",
                "read_file_head",
                "read_file_tail",
                "grep_search",
                "file_summary",
                "get_autonomic_catalog"
            ],
            "delegation_guideline": "Call route_task on EVERY turn. Call agent-brain verify -- <cmd> for tamper-evident test outcomes."
        }),
    ];

    if let Some(filter) = organ_filter {
        let f = filter.trim().to_ascii_lowercase();
        organs.retain(|o| {
            o.get("name")
                .and_then(|v| v.as_str())
                .map(|n| n.to_ascii_lowercase().contains(&f))
                .unwrap_or(false)
        });
    }

    serde_json::json!({
        "ecosystem": "autonomic-ai",
        "version": env!("CARGO_PKG_VERSION"),
        "total_organs": organs.len(),
        "organs": organs,
        "governance_rules": [
            "1. Route Task: Call agent-brain route_task at the beginning of each turn before using any other tools.",
            "2. Delegation: Prefer the tools listed in suggested_tools and the agent-spine workflows over native host improvisation.",
            "3. Durable Outcomes: Use agent-brain verify -- <cmd> to produce cryptographically signed proof in brain.db.",
            "4. Feedback: Store learned facts and decisions via store_memory."
        ],
        "verbose": verbose
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::store::{BrainStore, IndexBatchItem};

    #[test]
    fn proxy_handshake_replays_initialize_then_initialized_after_reconnect() {
        let mut handshake = ProxyHandshake::default();
        handshake.record(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#);
        handshake.record(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        handshake.record(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let replayed = handshake.replay();
        assert_eq!(replayed.len(), 2);
        assert!(replayed[0].contains(r#""method":"initialize""#));
        assert!(replayed[1].contains("notifications/initialized"));
    }

    #[test]
    fn proxy_handshake_replays_nothing_before_the_host_initializes() {
        let mut handshake = ProxyHandshake::default();
        handshake.record(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        assert!(handshake.replay().is_empty());
    }

    #[test]
    fn grep_search_params_accept_query_alias_and_default_path() {
        let params: GrepSearchParams =
            serde_json::from_value(serde_json::json!({ "query": "needle" }))
                .expect("query is accepted as the pattern");
        assert_eq!(params.pattern, "needle");
        assert_eq!(params.path, ".");
    }
    use crate::types::ItemType;
    use tempfile::TempDir;

    fn skill_test_server(dir: &TempDir, skills: &[IndexBatchItem]) -> BrainMcp {
        let home = dir.path().to_path_buf();
        std::fs::create_dir_all(home.join("data")).expect("create data dir");
        let config = crate::config::Config {
            home: home.clone(),
            data_dir: home.join("data"),
            logs_dir: home.join("logs"),
            db_path: home.join("data").join("brain.db"),
            vectors_path: home.join("data").join("vectors.bin"),
            turn_ttl_secs: 60,
            auto_capture_enabled: true,
            session_ingest_enabled: true,
            session_digest_enabled: true,
            session_ingest_legacy: false,
            session_max_age_days: 90,
            prewarm_on_bootstrap: false,
            bootstrap_background: false,
            embedding_cache_enabled: true,
            bm25_fast_path_enabled: true,
            session_ingest_background: false,
            turn_cache_ignore_open_files: true,
            embedding_model: "mini".into(),
            bootstrap_startup_delay_secs: 0,
            bootstrap_interval_secs: 0,
            auto_update_startup_delay_secs: 0,
            session_ingest_delay_secs: 0,
            session_ingest_route_interval_secs: 0,
            route_briefing_enabled: false,
            route_briefing_stderr: false,
            mcp_gate_enabled: true,
            mcp_gate_ttl_secs: 600,
            session_stickiness_secs: 0,
            precision_rerank_enabled: false,
            query_decompose_enabled: false,
            ann_enabled: true,
            ann_min_index: 1_500,
            ann_top_k: 100,
            auto_trajectory_enabled: false,
            workflow_dirs: vec![],
        };
        let store = Arc::new(BrainStore::open(&config.db_path).expect("open store"));
        store
            .upsert_indexed_items_batch(skills)
            .expect("seed indexed items");
        let engine = Arc::new(Engine::new_with_store(config, store).expect("engine"));
        BrainMcp::new(engine)
    }

    fn skill_row(topic: &str, text: &str, source_path: &str) -> IndexBatchItem {
        IndexBatchItem {
            id: format!("skill-{topic}"),
            item_type: ItemType::Skill,
            topic: topic.to_string(),
            text: text.to_string(),
            source_path: source_path.to_string(),
            scope: "global".to_string(),
            scope_key: None,
            content_hash: format!("hash-{topic}"),
            embedding: Vec::new(),
            file_mtime: None,
        }
    }

    fn json_body(result: CallToolResult) -> serde_json::Value {
        let text = result.content[0]
            .as_text()
            .expect("text content")
            .text
            .clone();
        serde_json::from_str(&text).expect("json body")
    }

    #[tokio::test]
    async fn skill_search_requires_route_first() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(
            &dir,
            &[skill_row(
                "rust-patterns",
                "rust ownership",
                "/skills/rust/SKILL.md",
            )],
        );

        let result = server
            .skill_search(Parameters(SkillSearchParams {
                query: "rust".into(),
                limit: 10,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn skill_search_returns_matching_skill_names_and_paths_after_routing() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(
            &dir,
            &[
                skill_row("rust-patterns", "rust ownership", "/skills/rust/SKILL.md"),
                skill_row("react-patterns", "hooks", "/skills/react/SKILL.md"),
            ],
        );
        server.route_gate.record_route("find a rust skill");

        let result = server
            .skill_search(Parameters(SkillSearchParams {
                query: "rust".into(),
                limit: 10,
            }))
            .await
            .expect("search succeeds");

        let body = json_body(result);
        let skills = body["skills"].as_array().expect("skills array");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["name"], "rust-patterns");
        assert_eq!(skills[0]["path"], "/skills/rust/SKILL.md");
    }

    #[tokio::test]
    async fn skill_search_rejects_query_without_words() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(&dir, &[]);
        server.route_gate.record_route("find a skill");

        let result = server
            .skill_search(Parameters(SkillSearchParams {
                query: "   ".into(),
                limit: 10,
            }))
            .await;

        let Err(err) = result else {
            panic!("blank query must be rejected");
        };
        assert_eq!(err.message, "query must contain at least one word");
    }

    #[tokio::test]
    async fn skill_get_returns_full_instructions_for_registry_name() {
        let dir = TempDir::new().expect("tempdir");
        let skill_path = dir.path().join("SKILL.md");
        std::fs::write(&skill_path, "# Rust Patterns\nUse ownership.").expect("write skill");
        let server = skill_test_server(
            &dir,
            &[skill_row(
                "rust-patterns",
                "rust ownership",
                &skill_path.to_string_lossy(),
            )],
        );
        server.route_gate.record_route("load rust patterns");

        let result = server
            .skill_get(Parameters(SkillGetParams {
                name: "RUST-PATTERNS".into(),
            }))
            .await
            .expect("skill_get succeeds");

        let body = json_body(result);
        assert_eq!(body["content"], "# Rust Patterns\nUse ownership.");
    }

    #[tokio::test]
    async fn skill_get_reports_unknown_skill_name() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(&dir, &[]);
        server.route_gate.record_route("load a skill");

        let result = server
            .skill_get(Parameters(SkillGetParams {
                name: "missing".into(),
            }))
            .await;

        let Err(err) = result else {
            panic!("unknown skill must be rejected");
        };
        assert_eq!(err.message, "skill not found: missing");
    }

    #[tokio::test]
    async fn skill_get_hides_indexed_path_when_file_is_gone() {
        let dir = TempDir::new().expect("tempdir");
        let gone = dir.path().join("gone").join("SKILL.md");
        let server = skill_test_server(
            &dir,
            &[skill_row("ghost", "ghost skill", &gone.to_string_lossy())],
        );
        server.route_gate.record_route("load ghost");

        let result = server
            .skill_get(Parameters(SkillGetParams {
                name: "ghost".into(),
            }))
            .await;

        let Err(err) = result else {
            panic!("missing file must be reported");
        };
        assert_eq!(err.message, "skill file is unavailable; rebuild the index");
        assert!(!err.message.contains(&*gone.to_string_lossy()));
    }

    fn upstream_tool(
        server: &str,
        name: &str,
        description: &str,
    ) -> crate::upstream::IndexedUpstreamTool {
        crate::upstream::IndexedUpstreamTool {
            server: server.to_string(),
            name: name.to_string(),
            description: description.to_string(),
        }
    }

    #[tokio::test]
    async fn mcp_search_requires_route_first() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(&dir, &[]);

        let result = server
            .mcp_search(Parameters(McpSearchParams {
                query: "issue".into(),
                limit: 10,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mcp_search_returns_tools_where_every_word_matches() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(&dir, &[]);
        server
            .engine
            .store
            .replace_upstream_tools(&[
                upstream_tool("linear", "create_issue", "Create an issue"),
                upstream_tool("linear", "list_teams", "List teams"),
            ])
            .expect("seed upstream tools");
        server.route_gate.record_route("create a linear issue");

        let result = server
            .mcp_search(Parameters(McpSearchParams {
                query: "LINEAR issue".into(),
                limit: 10,
            }))
            .await
            .expect("search succeeds");

        let body = json_body(result);
        let tools = body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["server"], "linear");
        assert_eq!(tools[0]["tool"], "create_issue");
    }

    #[tokio::test]
    async fn mcp_search_rejects_query_without_words() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(&dir, &[]);
        server.route_gate.record_route("find a tool");

        let result = server
            .mcp_search(Parameters(McpSearchParams {
                query: "  ".into(),
                limit: 10,
            }))
            .await;

        let Err(err) = result else {
            panic!("blank query must be rejected");
        };
        assert_eq!(err.message, "query must contain at least one word");
    }

    #[tokio::test]
    async fn get_mcp_reports_unknown_server_before_contacting_anything() {
        let dir = TempDir::new().expect("tempdir");
        let server = skill_test_server(&dir, &[]);
        server.route_gate.record_route("load a tool");

        let result = server
            .get_mcp(Parameters(GetMcpParams {
                server: "missing".into(),
                tool: "create_issue".into(),
            }))
            .await;

        let Err(err) = result else {
            panic!("unknown server must be rejected");
        };
        assert_eq!(err.message, "unknown or disabled upstream server: missing");
    }

    #[test]
    fn proxy_tracks_requests_until_their_response() {
        let request = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}"#;
        let response = r#"{"jsonrpc":"2.0","id":7,"result":{}}"#;
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert_eq!(request_id(request).as_deref(), Some("7"));
        assert_eq!(request_id(notification), None);
        assert_eq!(response_id(response).as_deref(), Some("7"));
        assert_eq!(response_id(request), None);
        assert_eq!(request_id("not json"), None);
    }

    #[test]
    fn proxy_timeout_answers_the_pending_request_with_an_error() {
        let reply: serde_json::Value = serde_json::from_str(&timeout_response("\"abc\"")).unwrap();
        assert_eq!(reply["id"], "abc");
        assert_eq!(reply["error"]["code"], -32000);
        assert!(reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("agent-brain doctor"));
    }

    #[test]
    fn test_build_autonomic_catalog_lists_only_in_scope_components() {
        let catalog = build_autonomic_catalog(None, false);
        assert_eq!(catalog["total_organs"], 2);
        let organs = catalog["organs"].as_array().unwrap();
        let names: Vec<_> = organs.iter().filter_map(|o| o["name"].as_str()).collect();
        assert!(names.contains(&"agent-spine"));
        assert!(names.contains(&"agent-brain"));
    }

    #[test]
    fn test_build_autonomic_catalog_filters_organ() {
        let catalog = build_autonomic_catalog(Some("spine"), false);
        assert_eq!(catalog["total_organs"], 1);
        assert_eq!(catalog["organs"][0]["name"], "agent-spine");
    }
}
