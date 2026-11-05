use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use agent_brain::config::Config;
use agent_brain::engine::Engine;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::jobs::{enqueue_job_detached, job_status, JobMode, JobTrigger};
use crate::query::query_codebase;
use crate::settings_or_default;

pub fn run(tool: &str, raw_args: &str) -> Result<()> {
    let args: Value = serde_json::from_str(raw_args).context("rpc arguments must be JSON")?;
    let config = Config::load()?;
    let home = config.home.clone();
    let engine = Arc::new(Engine::new(config)?);
    let bin = settings_or_default(&home).graphify_bin;
    match tool {
        "query_codebase" => {
            let repo = repo_arg(&args)?;
            let question = str_arg(&args, "question")?;
            let budget = args["max_tokens"].as_u64().unwrap_or(1500) as usize;
            let answer = query_codebase(&repo, &question, budget, &bin)?;
            emit(&json!({ "answer": answer }))
        }
        "trigger_deep_analysis" => {
            let repo = repo_arg(&args)?;
            let mode = match args["mode"].as_str() {
                Some("full") => JobMode::Full,
                _ => JobMode::Update,
            };
            let (job_id, worker) =
                enqueue_job_detached(&engine, &repo, JobTrigger::Agent, mode, &bin)?;
            emit(&json!({
                "job_id": job_id,
                "status": "queued",
                "reason": args["reason"],
            }))?;
            let _ = worker.join();
            Ok(())
        }
        "graphify_job_status" => {
            let status = job_status(&engine, &str_arg(&args, "job_id")?)?;
            emit(&serde_json::to_value(status)?)
        }
        other => bail!("unknown graphify tool: {other}"),
    }
}

fn repo_arg(args: &Value) -> Result<PathBuf> {
    let repo = str_arg(args, "repo")?;
    std::fs::canonicalize(&repo).with_context(|| format!("resolve repo root {repo}"))
}

fn str_arg(args: &Value, key: &str) -> Result<String> {
    args[key]
        .as_str()
        .map(str::to_string)
        .with_context(|| format!("missing string argument `{key}`"))
}

fn emit(value: &Value) -> Result<()> {
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{value}")?;
    stdout.flush()?;
    Ok(())
}
