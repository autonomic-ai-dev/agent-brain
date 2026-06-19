//! skills.sh catalog snapshot sync and production-scale routing eval.
//!
//! The skills.sh catalog has 700k+ skills; CI uses a committed snapshot of real skills
//! (default ~2000) indexed into fixture-2k.db. Sync uses public search/download APIs.
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
pub const SKILLS_SH_SEARCH_LIMIT: usize = 50;
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

pub fn default_discovery_queries() -> Vec<String> {
    vec![
        "react", "vue", "angular", "svelte", "nextjs", "node", "python", "rust", "go", "java",
        "kotlin", "swift", "flutter", "dart", "test", "tdd", "vitest", "jest", "playwright",
        "cypress", "deploy", "docker", "kubernetes", "terraform", "aws", "azure", "gcp",
        "security", "auth", "api", "graphql", "postgres", "mysql", "redis", "mongo", "supabase",
        "firebase", "stripe", "seo", "marketing", "design", "ui", "ux", "video", "audio", "pdf",
        "docx", "excel", "review", "debug", "plan", "mcp", "agent", "skill", "claude", "codex",
        "lark", "feishu", "github", "gitlab", "linear", "jira", "remotion", "animation", "browser",
        "scraping", "data", "ml", "ai", "writing", "copy", "email", "sales", "finance", "legal",
        "medical", "game", "mobile", "web", "css", "html", "tailwind", "shadcn", "prisma",
        "drizzle", "nestjs", "express", "fastapi", "django", "rails", "laravel", "php", "perl",
        "bash", "shell", "git", "commit", "refactor", "architecture", "performance",
        "accessibility", "i18n", "observability", "logging", "typescript", "javascript", "ruby",
        "elixir", "scala", "csharp", "dotnet", "wordpress", "shopify", "notion", "slack",
        "vercel", "netlify", "cloudflare", "openai", "anthropic", "gemini", "copilot", "figma",
        "sketch", "brand", "content", "social", "twitter", "linkedin", "docs", "tutorial",
        "onboarding", "testing", "lint", "format", "ci", "cd", "monitor", "sre", "devops",
        "infra", "network", "linux", "macos", "windows", "android", "ios", "react-native",
        "expo", "electron", "tauri", "wasm", "blockchain", "solidity", "ethereum", "crypto",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for SkillsShManifest {
    fn default() -> Self {
        Self {
            catalog_note: "skills.sh public catalog (730k+); sync --target 2000 for real fixture".into(),
            required_ids: vec![
                "vercel-labs/skills/find-skills".into(),
                "vercel-labs/agent-skills/vercel-react-best-practices".into(),
                "vercel-labs/agent-skills/vercel-react-native-skills".into(),
            ],
            discovery_queries: default_discovery_queries(),
            max_skills: SKILLS_SH_SIMULATED_INDEX,
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
    #[serde(default)]
    pub indexed_from: String,
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
    let mut manifest = if path.exists() {
        let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&raw).context("parse skills-sh manifest")?
    } else {
        SkillsShManifest::default()
    };
    if manifest.discovery_queries.len() < 50 {
        let mut seen = std::collections::BTreeSet::from_iter(manifest.discovery_queries.iter().cloned());
        for q in default_discovery_queries() {
            if seen.insert(q.clone()) {
                manifest.discovery_queries.push(q);
            }
        }
    }
    Ok(manifest)
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

#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub target: usize,
    pub delay_ms: u64,
    pub merge_path: Option<PathBuf>,
    pub checkpoint_path: Option<PathBuf>,
    pub checkpoint_every: usize,
    /// Max HTTP attempts per download (search keeps 5 for rate limits).
    pub download_max_attempts: u32,
}

impl SyncOptions {
    pub fn from_manifest(manifest: &SkillsShManifest, max_skills: Option<usize>, delay_ms: u64) -> Self {
        Self {
            target: max_skills.unwrap_or(manifest.max_skills),
            delay_ms,
            merge_path: None,
            checkpoint_path: None,
            checkpoint_every: 50,
            download_max_attempts: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub discovered_ids: usize,
    pub downloaded: usize,
    pub metadata_fallback: usize,
    pub skipped_existing: usize,
    pub failed: usize,
    pub total_in_snapshot: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetryFailedReport {
    pub attempted: usize,
    pub upgraded: usize,
    pub still_metadata: usize,
    pub total_in_snapshot: usize,
}

/// Re-download skills that were indexed from search metadata only.
pub fn retry_failed_downloads(
    snapshot_path: &Path,
    delay_ms: u64,
    download_max_attempts: u32,
    max_retries: Option<usize>,
) -> Result<(SkillsShSnapshot, RetryFailedReport)> {
    let mut snapshot = load_snapshot(snapshot_path)?;
    let mut upgraded = 0usize;
    let mut still_metadata = 0usize;
    let mut attempted = 0usize;

    for i in 0..snapshot.skills.len() {
        if snapshot.skills[i].indexed_from == "download" {
            continue;
        }
        if max_retries.is_some_and(|m| attempted >= m) {
            break;
        }
        attempted += 1;
        let skill_id = snapshot.skills[i].id.clone();
        let installs = snapshot.skills[i].installs;
        match download_skill(&skill_id, download_max_attempts) {
            Ok(mut record) => {
                record.installs = installs.or(record.installs);
                snapshot.skills[i] = record;
                upgraded += 1;
            }
            Err(err) => {
                tracing::warn!(skill_id = skill_id.as_str(), error = %err, "retry download failed");
                still_metadata += 1;
            }
        }
        if attempted % 50 == 0 {
            write_snapshot(snapshot_path, &snapshot)?;
            eprintln!("retry checkpoint: {} upgraded / {} attempted", upgraded, attempted);
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    write_snapshot(snapshot_path, &snapshot)?;
    let report = RetryFailedReport {
        attempted,
        upgraded,
        still_metadata,
        total_in_snapshot: snapshot.skills.len(),
    };
    Ok((snapshot, report))
}

/// Sync a snapshot from skills.sh (public search + download APIs).
pub fn sync_snapshot(
    manifest: &SkillsShManifest,
    max_skills: Option<usize>,
    delay_ms: u64,
) -> Result<SkillsShSnapshot> {
    sync_snapshot_with_options(manifest, SyncOptions::from_manifest(manifest, max_skills, delay_ms))
        .map(|(snapshot, _)| snapshot)
}

pub fn sync_snapshot_with_options(
    manifest: &SkillsShManifest,
    options: SyncOptions,
) -> Result<(SkillsShSnapshot, SyncReport)> {
    let cap = options.target;
    let mut existing: Vec<SkillsShSkillRecord> = if let Some(path) = &options.merge_path {
        if path.exists() {
            load_snapshot(path)?.skills
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let mut by_id: std::collections::BTreeMap<String, SkillsShSkillRecord> = existing
        .drain(..)
        .map(|s| (s.id.clone(), s))
        .collect();

    let mut meta_by_id: std::collections::BTreeMap<String, SearchSkill> = std::collections::BTreeMap::new();
    for id in &manifest.required_ids {
        meta_by_id.insert(
            id.clone(),
            SearchSkill {
                id: id.clone(),
                skill_id: None,
                name: id.rsplit('/').next().unwrap_or(id).to_string(),
                source: id.rsplit_once('/').map(|(s, _)| s.to_string()).unwrap_or_default(),
                installs: Some(u64::MAX),
            },
        );
    }

    for query in &manifest.discovery_queries {
        if meta_by_id.len() >= cap.saturating_mul(2) {
            break;
        }
        match search_skills(query, SKILLS_SH_SEARCH_LIMIT) {
            Ok(found) => {
                for skill in found {
                    meta_by_id.entry(skill.id.clone()).or_insert(skill);
                }
            }
            Err(err) => tracing::warn!(query = query.as_str(), error = %err, "skills.sh search failed"),
        }
        std::thread::sleep(Duration::from_millis(options.delay_ms));
    }

    let mut queue: Vec<SearchSkill> = meta_by_id.into_values().collect();
    queue.sort_by(|a, b| {
        b.installs
            .unwrap_or(0)
            .cmp(&a.installs.unwrap_or(0))
            .then_with(|| a.id.cmp(&b.id))
    });

    let discovered_ids = queue.len();
    let mut downloaded = 0usize;
    let mut metadata_fallback = 0usize;
    let mut skipped_existing = 0usize;
    let mut attempts = 0usize;

    for skill in queue {
        if by_id.len() >= cap {
            break;
        }
        if by_id.contains_key(&skill.id) {
            skipped_existing += 1;
            continue;
        }
        attempts += 1;
        match download_skill(&skill.id, options.download_max_attempts) {
            Ok(mut record) => {
                record.installs = skill.installs.or(record.installs);
                record.indexed_from = "download".into();
                by_id.insert(record.id.clone(), record);
                downloaded += 1;
            }
            Err(err) => {
                if manifest.required_ids.iter().any(|r| r == &skill.id) {
                    bail!("required skill {} download failed: {err}", skill.id);
                }
                tracing::warn!(skill_id = skill.id.as_str(), error = %err, "download failed; metadata fallback");
                let record = record_from_search_meta(&skill);
                by_id.insert(record.id.clone(), record);
                metadata_fallback += 1;
            }
        }
        if options.checkpoint_every > 0
            && attempts % options.checkpoint_every == 0
            && by_id.len() != skipped_existing
        {
            if let Some(path) = &options.checkpoint_path {
                let partial = snapshot_from_map(&by_id, cap);
                write_snapshot(path, &partial)?;
                eprintln!(
                    "checkpoint: {} skills in snapshot ({}/{})",
                    partial.skills.len(),
                    partial.skills.len(),
                    cap
                );
            }
        }
        std::thread::sleep(Duration::from_millis(options.delay_ms));
    }

    if by_id.len() < cap {
        tracing::warn!(
            have = by_id.len(),
            target = cap,
            "snapshot below target — add discovery queries or re-run with --merge"
        );
    }

    let snapshot = snapshot_from_map(&by_id, cap);
    let report = SyncReport {
        discovered_ids,
        downloaded,
        metadata_fallback,
        skipped_existing,
        failed: metadata_fallback,
        total_in_snapshot: snapshot.skills.len(),
    };
    Ok((snapshot, report))
}

fn snapshot_from_map(by_id: &std::collections::BTreeMap<String, SkillsShSkillRecord>, cap: usize) -> SkillsShSnapshot {
    let mut skills: Vec<SkillsShSkillRecord> = by_id.values().cloned().collect();
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    skills.truncate(cap);
    SkillsShSnapshot {
        generated_at: chrono::Utc::now().to_rfc3339(),
        source: "skills.sh".into(),
        catalog_size_note: format!(
            "{} real skills from skills.sh catalog (730k+); target index {}",
            skills.len(),
            cap
        ),
        skills,
    }
}

fn record_from_search_meta(skill: &SearchSkill) -> SkillsShSkillRecord {
    let slug = skill
        .skill_id
        .clone()
        .unwrap_or_else(|| skill.id.rsplit('/').next().unwrap_or(&skill.id).to_string());
    let topic = slug.clone();
    let text = format!(
        "{} {} {} skills.sh skill from {} installs {}",
        skill.name,
        topic,
        skill.source,
        skill.source,
        skill.installs.unwrap_or(0)
    );
    SkillsShSkillRecord {
        id: skill.id.clone(),
        slug,
        name: skill.name.clone(),
        source: skill.source.clone(),
        topic,
        text,
        installs: skill.installs,
        indexed_from: "search-metadata".into(),
    }
}

fn http_get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    http_get_json_attempts(url, 5)
}

fn download_http_agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::config::Config::builder()
            .timeout_global(Some(Duration::from_secs(3)))
            .build();
        ureq::Agent::new_with_config(config)
    })
}

fn http_get_json_attempts<T: serde::de::DeserializeOwned>(url: &str, max_attempts: u32) -> Result<T> {
    let agent = ureq::agent();
    http_get_json_with_agent(&agent, url, max_attempts)
}

fn http_get_json_with_agent<T: serde::de::DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
    max_attempts: u32,
) -> Result<T> {
    for attempt in 0..max_attempts {
        match agent.get(url).call() {
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
                if attempt + 1 == max_attempts {
                    return Err(err).with_context(|| format!("GET {url}"));
                }
                std::thread::sleep(Duration::from_millis(300 + attempt as u64 * 200));
            }
        }
    }
    bail!("GET {url} failed after {max_attempts} attempts")
}

fn search_skills(query: &str, limit: usize) -> Result<Vec<SearchSkill>> {
    let url = format!(
        "{SKILLS_SH_SEARCH_BASE}?q={}&limit={limit}",
        urlencoding(query)
    );
    let body: SearchResponse = http_get_json(&url)?;
    Ok(body.skills)
}

fn download_skill(id: &str, max_attempts: u32) -> Result<SkillsShSkillRecord> {
    let (source, slug) = parse_skill_id(id)?;
    let url = format!("{SKILLS_SH_DOWNLOAD_BASE}/{source}/{slug}");
    let body: DownloadResponse = http_get_json_with_agent(download_http_agent(), &url, max_attempts)?;
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
        indexed_from: "download".into(),
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

pub fn seed_fixture_index(
    store: &Arc<BrainStore>,
    snapshot: &SkillsShSnapshot,
    index_size: usize,
    allow_fillers: bool,
) -> Result<usize> {
    let indexed = index_snapshot(store, snapshot)?;
    if indexed >= index_size {
        return Ok(indexed.min(index_size));
    }
    if !allow_fillers {
        bail!(
            "snapshot has {} real skills but index target is {} — run: skills-sh sync --target {index_size} --merge",
            indexed,
            index_size
        );
    }
    let filler = index_size.saturating_sub(indexed);
    seed_filler_skills(store, filler)?;
    store.bump_index_version()?;
    Ok(indexed + filler)
}

/// Legacy name — uses fillers when snapshot is smaller than index_size.
pub fn seed_simulated_production_index(
    store: &Arc<BrainStore>,
    snapshot: &SkillsShSnapshot,
    index_size: usize,
) -> Result<usize> {
    seed_fixture_index(store, snapshot, index_size, true)
}

pub fn build_fixture_db(
    snapshot_path: &Path,
    index_size: usize,
    write_path: &Path,
    allow_fillers: bool,
) -> Result<FixtureBuildReport> {
    let snapshot = load_snapshot(snapshot_path)?;
    let dir = tempfile::tempdir()?;
    let config = crate::config::Config::isolated(dir.path().to_path_buf());
    config.ensure_dirs()?;
    let db_path = config.db_path.clone();
    let store = Arc::new(BrainStore::open(&db_path)?);
    let skills_to_index = if snapshot.skills.len() > index_size {
        SkillsShSnapshot {
            skills: snapshot.skills[..index_size].to_vec(),
            ..snapshot.clone()
        }
    } else {
        snapshot.clone()
    };
    let indexed = seed_fixture_index(&store, &skills_to_index, index_size, allow_fillers)?;
    stamp_fixture_meta(
        &store,
        FIXTURE_DB_KIND_SKILLS_SH,
        indexed,
        skills_to_index.skills.len(),
    )?;
    export_fixture_db(&store, &db_path, write_path)?;
    Ok(FixtureBuildReport {
        write_path: write_path.display().to_string(),
        index_size: indexed,
        snapshot_skills: skills_to_index.skills.len(),
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
            None,
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

pub fn write_golden(path: &Path, golden: &SkillsShGoldenFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(golden)?;
    std::fs::write(path, format!("{json}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn query_templates_for_skill(skill: &SkillsShSkillRecord) -> Vec<String> {
    let topic_words = skill.topic.replace('-', " ");
    let mut out = vec![
        format!("{topic_words} best practices and patterns"),
        format!("help me with {topic_words}"),
        format!("how to use {topic_words} in my project"),
    ];
    if !skill.name.is_empty() && skill.name != skill.topic {
        out.push(format!("{} workflow guidance", skill.name.replace('-', " ")));
    }
    let line = skill
        .text
        .lines()
        .find(|l| l.len() > 24 && l.len() < 180)
        .unwrap_or("")
        .trim()
        .to_string();
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn skill_probe_priority(skill: &SkillsShSkillRecord) -> u64 {
    let download_boost = if skill.indexed_from == "download" { 1_000_000 } else { 0 };
    let text_len = skill.text.len().min(50_000) as u64;
    download_boost + text_len + skill.installs.unwrap_or(0)
}

/// Probe routing queries against a fixture DB and return cases that hit Recall@3.
pub fn probe_golden_cases(
    fixture_db: &Path,
    snapshot_path: &Path,
    target: usize,
) -> Result<SkillsShGoldenFile> {
    let snapshot = load_snapshot(snapshot_path)?;
    let manifest = load_manifest(&default_manifest_path())?;
    let (engine, _dir) = open_fixture_engine(fixture_db)?;
    let limits = RouteLimits {
        agents: 0,
        skills: 3,
        rules: 0,
        memory: 0,
    };

    let mut skills: Vec<&SkillsShSkillRecord> = snapshot.skills.iter().collect();
    skills.sort_by_key(|s| std::cmp::Reverse(skill_probe_priority(s)));

    let required: std::collections::BTreeSet<String> =
        manifest.required_ids.iter().cloned().collect();
    skills.sort_by_key(|s| if required.contains(&s.id) { 0 } else { 1 });

    let mut cases = Vec::new();
    let mut seen_topics = std::collections::BTreeSet::new();

    for skill in skills {
        if cases.len() >= target {
            break;
        }
        if !seen_topics.insert(skill.topic.clone()) {
            continue;
        }
        for query in query_templates_for_skill(skill) {
            let resp = engine.route_task(
                &query,
                None,
                &[],
                500,
                limits,
                Some("implementing"),
                None,
            )?;
            let hit = resp
                .recommended_skills
                .iter()
                .take(3)
                .any(|s| s.name == skill.topic);
            if hit {
                cases.push(SkillsShGoldenCase {
                    query,
                    expected_topic: skill.topic.clone(),
                    skill_id: skill.id.clone(),
                });
                break;
            }
        }
    }

    Ok(SkillsShGoldenFile { cases })
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
