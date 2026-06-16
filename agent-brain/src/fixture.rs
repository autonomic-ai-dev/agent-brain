//! Isolated fixture DB for eval and benchmark proofs (never uses production `brain.db`).

use std::sync::Arc;

use anyhow::Result;
use tempfile::TempDir;

use crate::config::Config;
use crate::db::store::BrainStore;
use crate::engine::Engine;
use crate::eval::seed_eval_fixture;
use crate::types::RouteLimits;

pub const BENCH_FIXTURE_SKILLS: usize = 500;

/// Deterministic embedder + temp SQLite — same wiring as integration tests.
pub fn new_isolated_engine() -> Result<(Engine, TempDir)> {
    let dir = tempfile::tempdir()?;
    let mut config = Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs()?;
    let store = Arc::new(BrainStore::open(&config.db_path)?);
    let engine = Engine::new_with_store(config, store)?;
    Ok((engine, dir))
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
