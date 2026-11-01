use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use agent_brain::engine::Engine;

use crate::ingest::ingest_repo;
use crate::jobs::{enqueue_job, job_status, JobMode, JobTrigger};
use crate::repos::{disable_repo, enable_repo, list_repos, repo_status};
use crate::settings_or_default;

#[derive(Debug, Clone)]
pub struct GraphifyCli {
    pub sub: String,
    pub repo: Option<PathBuf>,
    pub trigger: Option<String>,
    pub mode: Option<String>,
    pub job_id: Option<String>,
    pub question: Option<String>,
    pub budget: usize,
}

pub fn run_cli(engine: &Arc<Engine>, cli: GraphifyCli) -> Result<()> {
    let settings = settings_or_default(&engine.config.home);
    let graphify_bin = settings.graphify_bin.clone();
    match cli.sub.as_str() {
        "enable" => {
            let repo = cli
                .repo
                .as_deref()
                .context("--repo required for graphify enable")?;
            let rec = enable_repo(&engine.config.home, repo, &graphify_bin)?;
            println!("{}", serde_json::to_string_pretty(&rec)?);
        }
        "disable" => {
            let repo = cli
                .repo
                .as_deref()
                .context("--repo required for graphify disable")?;
            disable_repo(&engine.config.home, repo)?;
            println!("disabled graphify for {}", repo.display());
        }
        "status" => {
            let rows = repo_status(&engine.config.home, cli.repo.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        "list" => {
            let rows = list_repos(&engine.config.home)?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        "ingest" => {
            let repo = resolve_repo(cli.repo.as_deref())?;
            let report = ingest_repo(&engine.store, &engine.config.home, &repo)?;
            println!(
                "ingested {} nodes, {} edges ({} god nodes)",
                report.nodes, report.edges, report.god_nodes
            );
        }
        "run" => {
            let repo = resolve_repo(cli.repo.as_deref())?;
            let mode = if cli.mode.as_deref() == Some("full") {
                JobMode::Full
            } else {
                JobMode::Update
            };
            let job_id = enqueue_job(engine, &repo, JobTrigger::Manual, mode, &graphify_bin)?;
            println!("queued graphify job {job_id}");
        }
        "job" => {
            let job_id = cli
                .job_id
                .as_deref()
                .context("graphify job requires --id")?;
            let status = job_status(engine, job_id)?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        "query" => {
            let repo = resolve_repo(cli.repo.as_deref())?;
            let question = cli
                .question
                .as_deref()
                .context("--question required for graphify query")?;
            let text = crate::query::query_codebase(&repo, question, cli.budget, &graphify_bin)?;
            print!("{text}");
        }
        other => bail!("unknown graphify subcommand: {other}"),
    }
    Ok(())
}

fn resolve_repo(repo: Option<&Path>) -> Result<PathBuf> {
    let repo = repo.context("--repo required (or run from project root with graphify enabled)")?;
    let abs = if repo.is_absolute() {
        repo.to_path_buf()
    } else {
        std::env::current_dir()?.join(repo)
    };
    std::fs::canonicalize(abs).context("resolve repo path")
}
