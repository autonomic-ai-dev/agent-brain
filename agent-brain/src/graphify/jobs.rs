use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::db::store::BrainStore;
use crate::engine::Engine;

use super::ingest::ingest_repo;
use super::process::output_with_timeout;
use super::types::GraphifyJobStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobTrigger {
    GitHook,
    Agent,
    Idle,
    Manual,
}

impl JobTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHook => "git_hook",
            Self::Agent => "agent",
            Self::Idle => "idle",
            Self::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "git_hook" => Some(Self::GitHook),
            "agent" => Some(Self::Agent),
            "idle" => Some(Self::Idle),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobMode {
    IngestOnly,
    Update,
    Full,
}

impl JobMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IngestOnly => "ingest_only",
            Self::Update => "update",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ingest_only" => Some(Self::IngestOnly),
            "update" => Some(Self::Update),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

static JOB_LOCK: Mutex<()> = Mutex::new(());

const GRAPHIFY_JOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub fn enqueue_job(
    engine: &Arc<Engine>,
    repo_root: &Path,
    trigger: JobTrigger,
    mode: JobMode,
    graphify_bin: &str,
) -> Result<String> {
    let job_id = Uuid::new_v4().to_string();
    let repo_str = repo_root.display().to_string();
    engine.store.insert_graphify_job(
        &job_id,
        &repo_str,
        trigger.as_str(),
        mode.as_str(),
        "queued",
        None,
        None,
        None,
    )?;
    let engine = Arc::clone(engine);
    let repo = repo_root.to_path_buf();
    let bin = graphify_bin.to_string();
    let job_id_spawn = job_id.clone();
    thread::spawn(move || {
        if let Err(err) = run_job(&engine, &job_id_spawn, &repo, trigger, mode, &bin) {
            tracing::warn!(job_id = %job_id_spawn, error = %err, "graphify job failed");
        }
    });
    Ok(job_id)
}

fn run_job(
    engine: &Engine,
    job_id: &str,
    repo_root: &Path,
    _trigger: JobTrigger,
    mode: JobMode,
    graphify_bin: &str,
) -> Result<()> {
    let _guard = JOB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let started = chrono::Utc::now().timestamp();
    engine
        .store
        .update_graphify_job(job_id, "running", Some(started), None, None, None)?;

    let result = match mode {
        JobMode::IngestOnly => run_ingest_only(engine, repo_root),
        JobMode::Update => run_graphify_then_ingest(engine, repo_root, graphify_bin, true),
        JobMode::Full => run_graphify_then_ingest(engine, repo_root, graphify_bin, false),
    };

    let finished = chrono::Utc::now().timestamp();
    match result {
        Ok(report) => {
            let payload = serde_json::json!({
                "nodes": report.nodes,
                "edges": report.edges,
                "god_nodes": report.god_nodes,
                "graph_path": repo_root.join("graphify-out").join("graph.json").display().to_string(),
            });
            engine.store.update_graphify_job(
                job_id,
                "done",
                Some(started),
                Some(finished),
                None,
                Some(&payload.to_string()),
            )?;
        }
        Err(err) => {
            engine.store.update_graphify_job(
                job_id,
                "failed",
                Some(started),
                Some(finished),
                Some(&err.to_string()),
                None,
            )?;
            return Err(err);
        }
    }
    Ok(())
}

fn run_ingest_only(engine: &Engine, repo_root: &Path) -> Result<super::ingest::IngestReport> {
    ingest_repo(&engine.store, &engine.config.home, repo_root)
}

fn run_graphify_then_ingest(
    engine: &Engine,
    repo_root: &Path,
    graphify_bin: &str,
    incremental: bool,
) -> Result<super::ingest::IngestReport> {
    let graph_path = repo_root.join("graphify-out").join("graph.json");
    let mut cmd = Command::new(graphify_bin);
    cmd.current_dir(repo_root)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if incremental && graph_path.exists() {
        cmd.arg("--update");
    }
    cmd.arg(".");
    let output = output_with_timeout(&mut cmd, GRAPHIFY_JOB_TIMEOUT)
        .with_context(|| format!("run {graphify_bin}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("graphify failed: {stderr}");
    }
    // graphify may take time writing outputs
    for _ in 0..10 {
        if graph_path.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    ingest_repo(&engine.store, &engine.config.home, repo_root)
}

pub fn job_status(engine: &Engine, job_id: &str) -> Result<GraphifyJobStatus> {
    let rec = engine
        .store
        .get_graphify_job(job_id)?
        .with_context(|| format!("unknown graphify job {job_id}"))?;
    let status = rec.status.clone();
    let result = rec
        .result_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok());
    Ok(GraphifyJobStatus {
        job_id: rec.id,
        status: status.clone(),
        progress: if status == "running" {
            Some("graphify pipeline".into())
        } else {
            None
        },
        error: rec.error,
        result,
    })
}

impl BrainStore {
    pub fn insert_graphify_job(
        &self,
        id: &str,
        repo_root: &str,
        trigger: &str,
        mode: &str,
        status: &str,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                INSERT INTO graphify_jobs (
                    id, repo_root, trigger, mode, status, started_at, finished_at, error, result_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)
                "#,
                rusqlite::params![
                    id,
                    repo_root,
                    trigger,
                    mode,
                    status,
                    started_at,
                    finished_at,
                    error
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_graphify_job(
        &self,
        id: &str,
        status: &str,
        started_at: Option<i64>,
        finished_at: Option<i64>,
        error: Option<&str>,
        result_json: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
                UPDATE graphify_jobs
                SET status = ?2,
                    started_at = COALESCE(?3, started_at),
                    finished_at = COALESCE(?4, finished_at),
                    error = COALESCE(?5, error),
                    result_json = COALESCE(?6, result_json)
                WHERE id = ?1
                "#,
                rusqlite::params![id, status, started_at, finished_at, error, result_json],
            )?;
            Ok(())
        })
    }

    pub fn get_graphify_job(&self, id: &str) -> Result<Option<super::types::GraphifyJobRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, repo_root, trigger, mode, status, started_at, finished_at, error, result_json FROM graphify_jobs WHERE id = ?1",
            )?;
            let mut rows = stmt.query([id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(super::types::GraphifyJobRecord {
                    id: row.get(0)?,
                    repo_root: row.get(1)?,
                    trigger: row.get(2)?,
                    mode: row.get(3)?,
                    status: row.get(4)?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    error: row.get(7)?,
                    result_json: row.get(8)?,
                }))
            } else {
                Ok(None)
            }
        })
    }
}
