//! Isolated fixture DB for eval and benchmark proofs (never uses production `brain.db`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::config::Config;
use crate::db::store::BrainStore;
use crate::embed::{parse_embedding_model, Embedder};
use crate::engine::Engine;
use crate::eval::seed_eval_fixture;
use crate::types::RouteLimits;

pub const BENCH_FIXTURE_SKILLS: usize = 500;

/// Bump when the fixture **build recipe** changes (not SQLite schema migrations).
pub const FIXTURE_DB_RECIPE_VERSION: &str = "2";
pub const FIXTURE_DB_KIND_SKILLS_SH: &str = "skills-sh";

/// Deterministic embedder + temp SQLite — same wiring as integration tests.
pub fn new_isolated_engine() -> Result<(Engine, TempDir)> {
    let dir = tempfile::tempdir()?;
    let config = Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs()?;
    let store = Arc::new(BrainStore::open(&config.db_path)?);
    let engine = Engine::new_with_store(config, store)?;
    Ok((engine, dir))
}

/// Open a committed fixture DB (copied to a temp dir so route_task can write logs).
pub fn open_fixture_engine(fixture_path: &Path) -> Result<(Engine, TempDir)> {
    open_fixture_engine_with_onnx(fixture_path, false)
}

/// Open fixture DB with ONNX query embedder (indexed rows keep baked deterministic vectors).
pub fn open_fixture_engine_with_onnx(fixture_path: &Path, onnx: bool) -> Result<(Engine, TempDir)> {
    let meta = read_fixture_meta(fixture_path)?;
    let dir = tempfile::tempdir()?;
    let config = Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs()?;
    std::fs::copy(fixture_path, &config.db_path)
        .with_context(|| format!("copy fixture {}", fixture_path.display()))?;
    let store = Arc::new(BrainStore::open(&config.db_path)?);
    verify_fixture_store(&store, &meta)?;
    let embedder = if onnx {
        let model = parse_embedding_model(&config.embedding_model);
        Arc::new(Embedder::with_model(model)?)
    } else {
        Arc::new(Embedder::deterministic())
    };
    let engine = Engine::new_with_store_and_embedder(config, store, embedder)?;
    Ok((engine, dir))
}

fn benchmark_path(rel: &str) -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(repo) = crate::config::find_repo_root(&cwd) {
            return repo.join(rel);
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel)
}

pub fn default_fixture_2k_path() -> PathBuf {
    benchmark_path("docs/benchmarks/fixture-2k.db")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureDbMeta {
    pub kind: String,
    pub recipe_version: String,
    pub index_size: usize,
    pub snapshot_skills: usize,
    pub filler_skills: usize,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureDbBreakdown {
    pub total_indexed: usize,
    pub snapshot_skills: usize,
    pub filler_skills: usize,
    pub skills_sh_rows: usize,
    pub bench_filler_rows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureBuildReport {
    pub write_path: String,
    pub index_size: usize,
    pub snapshot_skills: usize,
    pub recipe_version: String,
}

pub fn stamp_fixture_meta(
    store: &BrainStore,
    kind: &str,
    index_size: usize,
    snapshot_skills: usize,
) -> Result<()> {
    store.set_meta("fixture_kind", kind)?;
    store.set_meta("fixture_recipe_version", FIXTURE_DB_RECIPE_VERSION)?;
    store.set_meta("fixture_index_size", &index_size.to_string())?;
    store.set_meta("fixture_snapshot_skills", &snapshot_skills.to_string())?;
    store.set_meta("fixture_generated_at", &chrono::Utc::now().to_rfc3339())?;
    store.set_meta("embedding_model", "deterministic")?;
    Ok(())
}

/// Checkpoint WAL and write a portable single-file SQLite DB.
pub fn export_fixture_db(store: &BrainStore, source_db_path: &Path, write_path: &Path) -> Result<()> {
    store.checkpoint_wal()?;
    if let Some(parent) = write_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source_db_path, write_path)
        .with_context(|| format!("export fixture to {}", write_path.display()))?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", write_path.display()));
        if sidecar.exists() {
            std::fs::remove_file(&sidecar).ok();
        }
    }
    Ok(())
}

pub fn read_fixture_meta(fixture_path: &Path) -> Result<FixtureDbMeta> {
    let dir = tempfile::tempdir()?;
    let config = Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs()?;
    std::fs::copy(fixture_path, &config.db_path)?;
    let store = BrainStore::open(&config.db_path)?;
    let kind = store
        .get_meta("fixture_kind")?
        .context("missing fixture_kind meta — not a benchmark fixture db")?;
    let recipe_version = store
        .get_meta("fixture_recipe_version")?
        .unwrap_or_default();
    let index_size = store
        .get_meta("fixture_index_size")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let snapshot_skills = store
        .get_meta("fixture_snapshot_skills")?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let generated_at = store
        .get_meta("fixture_generated_at")?
        .unwrap_or_default();
    Ok(FixtureDbMeta {
        kind,
        recipe_version,
        index_size,
        snapshot_skills,
        filler_skills: index_size.saturating_sub(snapshot_skills),
        generated_at,
    })
}

pub fn fixture_db_breakdown(store: &BrainStore) -> Result<FixtureDbBreakdown> {
    let total_indexed = store.count_indexed_items()?;
    let skills_sh_rows = store.count_indexed_items_matching(
        "SELECT COUNT(*) FROM indexed_items WHERE source_path LIKE 'https://skills.sh/%'",
    )?;
    let bench_filler_rows = store.count_indexed_items_matching(
        "SELECT COUNT(*) FROM indexed_items WHERE topic LIKE 'bench-filler-%'",
    )?;
    Ok(FixtureDbBreakdown {
        total_indexed,
        snapshot_skills: skills_sh_rows,
        filler_skills: bench_filler_rows,
        skills_sh_rows,
        bench_filler_rows,
    })
}

fn verify_fixture_store(store: &BrainStore, meta: &FixtureDbMeta) -> Result<()> {
    if meta.recipe_version != FIXTURE_DB_RECIPE_VERSION {
        bail!(
            "fixture recipe version {} != expected {} — rebuild with `agent-brain fixture build`",
            meta.recipe_version,
            FIXTURE_DB_RECIPE_VERSION
        );
    }
    let count = store.count_indexed_items()?;
    if count != meta.index_size {
        bail!(
            "fixture index size mismatch: meta says {} but indexed_items has {}",
            meta.index_size,
            count
        );
    }
    Ok(())
}

/// Golden eval seeds only (9 skills + 4 facts).
pub fn seed_eval_only(store: &Arc<BrainStore>) -> Result<()> {
    seed_eval_fixture(store)
}

/// Eval golden set plus filler skills to stress hybrid scoring at scale.
pub fn seed_bench_fixture(store: &Arc<BrainStore>, total_skills: usize) -> Result<usize> {
    seed_eval_fixture(store)?;
    let golden_and_decoys = 9usize;
    let filler = total_skills.saturating_sub(golden_and_decoys);
    crate::eval::seed_filler_skills(store, filler)?;
    store.bump_index_version()?;
    Ok(golden_and_decoys + filler)
}

pub fn default_route_limits() -> RouteLimits {
    RouteLimits {
        agents: 2,
        skills: 3,
        rules: 3,
        memory: 5,
    }
}

pub type IsolatedEvalReport = crate::eval::EvalReport;
