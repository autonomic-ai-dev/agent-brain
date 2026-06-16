//! skills.sh catalog snapshot sync and production-scale routing eval.
//!
//! The skills.sh catalog has 700k+ skills; CI uses a committed snapshot plus filler
//! skills to simulate a ~2000-item production index. Sync uses public search/download APIs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::db::store::{content_hash, BrainStore};
use crate::embed::deterministic_embedding;
use crate::eval::seed_filler_skills;
use crate::fixture::{
    export_fixture_db, new_isolated_engine, open_fixture_engine, stamp_fixture_meta,
    FixtureBuildReport, FIXTURE_DB_KIND_SKILLS_SH,
};
use crate::index::skill_index_text;
use crate::types::{ItemType, RouteLimits};

pub const SKILLS_SH_RECALL_THRESHOLD: f64 = 0.80;
pub const SKILLS_SH_SIMULATED_INDEX: usize = 2000;
pub const SKILLS_SH_SEARCH_BASE: &str = "https://skills.sh/api/search";
pub const SKILLS_SH_DOWNLOAD_BASE: &str = "https://skills.sh/api/download";
pub const SKILLS_SH_V1_BASE: &str = "https://skills.sh/api/v1/skills";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShManifest {
    pub catalog_note: String,
    pub required_ids: Vec<String>,
    pub discovery_queries: Vec<String>,
    pub max_skills: usize,
}

impl Default for SkillsShManifest {
    fn default() -> Self {
        Self {
            catalog_note: "skills.sh public catalog (730k+); snapshot is a searchable sample".into(),
            required_ids: vec![
                "vercel-labs/skills/find-skills".into(),
                "vercel-labs/agent-skills/vercel-react-best-practices".into(),
                "vercel-labs/agent-skills/vercel-react-native-skills".into(),
                "expo/skills/expo-deployment".into(),
                "supabase/agent-skills/supabase-postgres-best-practices".into(),
            ],
            discovery_queries: vec![
                "react".into(),
                "nextjs".into(),
                "deploy".into(),
                "test".into(),
                "review".into(),
                "debug".into(),
                "rust".into(),
                "python".into(),
                "postgres".into(),
                "docker".into(),
                "security".into(),
                "typescript".into(),
                "vitest".into(),
                "playwright".into(),
                "kubernetes".into(),
                "mcp".into(),
                "plan".into(),
                "api".into(),
            ],
            max_skills: 200,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShSnapshot {
    pub generated_at: String,
    pub source: String,
    pub catalog_size_note: String,
    pub skills: Vec<SkillsShSkillRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShSkillRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub source: String,
    pub topic: String,
    pub text: String,
    pub installs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShGoldenFile {
    pub cases: Vec<SkillsShGoldenCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShGoldenCase {
    pub query: String,
    pub expected_topic: String,
    pub skill_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShEvalReport {
    pub snapshot_skills: usize,
    pub filler_skills: usize,
    pub simulated_index_size: usize,
    #[serde(default)]
    pub fixture_db: Option<String>,
    #[serde(default)]
    pub index_mode: String,
    pub cases: usize,
    pub passed: usize,
    pub recall_at_3: f64,
    pub threshold: f64,
    pub failures: Vec<SkillsShEvalFailure>,
    pub passed_gate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShEvalFailure {
    pub query: String,
    pub expected_topic: String,
    pub skill_id: String,
    pub got_topics: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    skills: Vec<SearchSkill>,
}

#[derive(Debug, Deserialize)]
struct SearchSkill {
    id: String,
    #[serde(alias = "skillId")]
    skill_id: Option<String>,
    name: String,
    source: String,
    installs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    files: Vec<DownloadFile>,
}

#[derive(Debug, Deserialize)]
struct DownloadFile {
    path: String,
    contents: String,
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

pub fn default_manifest_path() -> PathBuf {
    benchmark_path("docs/benchmarks/skills-sh/manifest.json")
}

pub fn default_snapshot_path() -> PathBuf {
    benchmark_path("docs/benchmarks/skills-sh/snapshot.json")
}

pub fn default_golden_path() -> PathBuf {
    benchmark_path("docs/benchmarks/skills-sh/golden.json")
}

pub fn default_skills_sh_report_path() -> PathBuf {
    benchmark_path("docs/benchmarks/skills-sh-latest.json")
}

pub fn load_manifest(path: &Path) -> Result<SkillsShManifest> {
    if path.exists() {
        let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        return Ok(serde_json::from_str(&raw).context("parse skills-sh manifest")?);
    }
    Ok(SkillsShManifest::default())
}

pub fn load_snapshot(path: &Path) -> Result<SkillsShSnapshot> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).context("parse skills-sh snapshot")
}

pub fn load_golden(path: &Path) -> Result<SkillsShGoldenFile> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).context("parse skills-sh golden")
}

pub fn write_snapshot(path: &Path, snapshot: &SkillsShSnapshot) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(path, format!("{json}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Sync a snapshot from skills.sh (public search + download APIs).
pub fn sync_snapshot(
    manifest: &SkillsShManifest,
    max_skills: Option<usize>,
    delay_ms: u64,
) -> Result<SkillsShSnapshot> {
    let cap = max_skills.unwrap_or(manifest.max_skills);
    let mut ids: BTreeSet<String> = manifest.required_ids.iter().cloned().collect();

    for query in &manifest.discovery_queries {
        if ids.len() >= cap {
            break;
        }
        match search_skills(query, 50) {
            Ok(found) => {
                for skill in found {
                    ids.insert(skill.id);
                    if ids.len() >= cap {
                        break;
                    }
                }
            }
            Err(err) => tracing::warn!(query = query.as_str(), error = %err, "skills.sh search failed"),
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    let mut skills = Vec::new();
    for id in ids.iter().take(cap) {
        match download_skill(id) {
            Ok(record) => skills.push(record),
            Err(err) => {
                if manifest.required_ids.iter().any(|r| r == id) {
                    bail!("required skill {id} download failed: {err}");
                }
                tracing::warn!(skill_id = id.as_str(), error = %err, "skip skill download");
            }
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(SkillsShSnapshot {
        generated_at: chrono::Utc::now().to_rfc3339(),
        source: "skills.sh".into(),
        catalog_size_note: "730k+ skills in catalog; snapshot is a reproducible sample for CI".into(),
        skills,
    })
}

fn http_get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    const MAX_ATTEMPTS: u32 = 5;
    for attempt in 0..MAX_ATTEMPTS {
        match ureq::get(url).call() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status == 429 {
                    let wait = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(5 + attempt as u64 * 10);
                    tracing::warn!(url, wait_secs = wait, "rate limited, backing off");
                    std::thread::sleep(Duration::from_secs(wait));
                    continue;
                }
                if !(200..300).contains(&status) {
                    bail!("GET {url} returned {status}");
                }
                return Ok(resp.into_body().read_json()?);
            }
            Err(ureq::Error::StatusCode(code)) if code == 429 => {
                let wait = 5 + attempt as u64;
                std::thread::sleep(Duration::from_secs(wait));
            }
            Err(err) => {
                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(err).with_context(|| format!("GET {url}"));
                }
                std::thread::sleep(Duration::from_secs(2 + attempt as u64));
            }
        }
    }
    bail!("GET {url} failed after {MAX_ATTEMPTS} attempts")
}

fn search_skills(query: &str, limit: usize) -> Result<Vec<SearchSkill>> {
    let url = format!(
        "{SKILLS_SH_SEARCH_BASE}?q={}&limit={limit}",
        urlencoding(query)
    );
    let body: SearchResponse = http_get_json(&url)?;
    Ok(body.skills)
}

fn download_skill(id: &str) -> Result<SkillsShSkillRecord> {
    let (source, slug) = parse_skill_id(id)?;
    let url = format!("{SKILLS_SH_DOWNLOAD_BASE}/{source}/{slug}");
    let body: DownloadResponse = http_get_json(&url)?;
    let skill_md = body
        .files
        .iter()
        .find(|f| f.path.eq_ignore_ascii_case("SKILL.md"))
        .map(|f| f.contents.as_str())
        .context("SKILL.md missing in download response")?;
    let topic = slug.to_string();
    let text = skill_index_text(skill_md, &topic);
    Ok(SkillsShSkillRecord {
        id: id.to_string(),
        slug: slug.to_string(),
        name: topic.clone(),
        source,
        topic,
        text,
        installs: None,
    })
}

fn parse_skill_id(id: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() < 2 {
        bail!("invalid skill id: {id}");
    }
    let slug = parts.last().unwrap().to_string();
    let source = parts[..parts.len() - 1].join("/");
    Ok((source, slug))
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn index_snapshot(store: &Arc<BrainStore>, snapshot: &SkillsShSnapshot) -> Result<usize> {
    let mut count = 0usize;
    for skill in &snapshot.skills {
        let emb = deterministic_embedding(&format!("{} {}", skill.topic, skill.text));
        let hash = content_hash(&skill.text);
        store.upsert_indexed_item(
            ItemType::Skill,
            &skill.topic,
            &skill.text,
            &format!("https://skills.sh/{}", skill.id),
            "global",
            None,
            &hash,
            Some(&emb),
        )?;
        count += 1;
    }
    store.bump_index_version()?;
    Ok(count)
}

pub fn seed_simulated_production_index(
    store: &Arc<BrainStore>,
    snapshot: &SkillsShSnapshot,
    index_size: usize,
) -> Result<usize> {
    let indexed = index_snapshot(store, snapshot)?;
    let filler = index_size.saturating_sub(indexed);
    if filler > 0 {
        seed_filler_skills(store, filler)?;
        store.bump_index_version()?;
    }
    Ok(indexed + filler)
}

pub fn build_fixture_db(
    snapshot_path: &Path,
    index_size: usize,
    write_path: &Path,
) -> Result<FixtureBuildReport> {
    let snapshot = load_snapshot(snapshot_path)?;
    let dir = tempfile::tempdir()?;
    let config = crate::config::Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs()?;
    let db_path = config.db_path.clone();
    let store = Arc::new(BrainStore::open(&db_path)?);
    let total = seed_simulated_production_index(&store, &snapshot, index_size)?;
    stamp_fixture_meta(
        &store,
        FIXTURE_DB_KIND_SKILLS_SH,
        total,
        snapshot.skills.len(),
    )?;
    export_fixture_db(&store, &db_path, write_path)?;
    Ok(FixtureBuildReport {
        write_path: write_path.display().to_string(),
        index_size: total,
        snapshot_skills: snapshot.skills.len(),
        recipe_version: crate::fixture::FIXTURE_DB_RECIPE_VERSION.into(),
    })
}

fn run_skills_sh_eval_on_engine(
    engine: &crate::engine::Engine,
    golden: &SkillsShGoldenFile,
    snapshot_skills: usize,
    index_size: usize,
    fixture_db: Option<&Path>,
    index_mode: &str,
) -> Result<SkillsShEvalReport> {
    let limits = RouteLimits {
        agents: 0,
        skills: 3,
        rules: 0,
        memory: 0,
    };

    let mut passed = 0usize;
    let mut failures = Vec::new();

    for case in &golden.cases {
        let resp = engine.route_task(
            &case.query,
            None,
            &[],
            500,
            limits,
            Some("implementing"),
        )?;
        let got: Vec<String> = resp
            .recommended_skills
            .iter()
            .take(3)
            .map(|s| s.name.clone())
            .collect();
        if got.iter().any(|t| t == &case.expected_topic) {
            passed += 1;
        } else {
            failures.push(SkillsShEvalFailure {
                query: case.query.clone(),
                expected_topic: case.expected_topic.clone(),
                skill_id: case.skill_id.clone(),
                got_topics: got,
            });
        }
    }

    let cases = golden.cases.len();
    let recall_at_3 = if cases == 0 {
        1.0
    } else {
        passed as f64 / cases as f64
    };
    let passed_gate = recall_at_3 >= SKILLS_SH_RECALL_THRESHOLD;

    Ok(SkillsShEvalReport {
        snapshot_skills,
        filler_skills: index_size.saturating_sub(snapshot_skills),
        simulated_index_size: index_size,
        fixture_db: fixture_db.map(|p| p.display().to_string()),
        index_mode: index_mode.into(),
        cases,
        passed,
        recall_at_3,
        threshold: SKILLS_SH_RECALL_THRESHOLD,
        failures,
        passed_gate,
    })
}

pub fn run_skills_sh_eval(
    snapshot_path: &Path,
    golden_path: &Path,
    fixture_db: Option<&Path>,
) -> Result<SkillsShEvalReport> {
    let golden = load_golden(golden_path)?;

    if let Some(db_path) = fixture_db {
        let (engine, _dir) = open_fixture_engine(db_path)?;
        let meta = crate::fixture::read_fixture_meta(db_path)?;
        return run_skills_sh_eval_on_engine(
            &engine,
            &golden,
            meta.snapshot_skills,
            meta.index_size,
            Some(db_path),
            "fixture-db",
        );
    }

    let snapshot = load_snapshot(snapshot_path)?;
    let (engine, _dir) = new_isolated_engine()?;
    let total = seed_simulated_production_index(&engine.store, &snapshot, SKILLS_SH_SIMULATED_INDEX)?;
    run_skills_sh_eval_on_engine(
        &engine,
        &golden,
        snapshot.skills.len(),
        total,
        None,
        "runtime-seed",
    )
}

pub fn assert_skills_sh_gate(report: &SkillsShEvalReport) -> Result<()> {
    if report.passed_gate {
        return Ok(());
    }
    bail!(
        "skills.sh Recall@3 {:.2} below threshold {:.2} ({} / {} passed)",
        report.recall_at_3,
        report.threshold,
        report.passed,
        report.cases
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_id_splits_source_and_slug() {
        let (source, slug) =
            parse_skill_id("vercel-labs/agent-skills/vercel-react-best-practices").unwrap();
        assert_eq!(source, "vercel-labs/agent-skills");
        assert_eq!(slug, "vercel-react-best-practices");
    }
}
