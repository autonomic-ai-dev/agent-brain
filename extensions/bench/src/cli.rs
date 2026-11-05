use std::path::{Path, PathBuf};

use agent_brain::config::Config;
use anyhow::Result;

pub fn dispatch(cmd: &str, args: &[String]) -> Result<()> {
    match cmd {
        "bench" => {
            let onnx = args.iter().any(|a| a == "--onnx");
            let ci = args.iter().any(|a| a == "--ci");
            let supervisor = args.iter().any(|a| a == "--supervisor");
            let scale = args.iter().any(|a| a == "--scale");
            let graphify = args.iter().any(|a| a == "--graphify");
            let mcp = args.iter().any(|a| a == "--mcp");
            let beam = args.iter().any(|a| a == "--beam");
            if beam {
                let report = crate::beam_eval::run_beam_eval_isolated()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Err(err) = crate::beam_eval::assert_beam_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            } else if mcp {
                let report = crate::mcp_bench::run_mcp_bench()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) = crate::mcp_bench::assert_mcp_bench_gate(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if graphify {
                let full = args.iter().any(|a| a == "--full");
                let sizes: &[usize] = if full {
                    crate::graphify_bench::GRAPHIFY_SIZES
                } else {
                    &[1_000]
                };
                let report = crate::graphify_bench::run_graphify_bench(sizes)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) = crate::graphify_bench::assert_graphify_bench_gate(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if supervisor {
                let report = crate::supervisor_bench::run_supervisor_bench()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) = crate::supervisor_bench::assert_supervisor_bench_gate(&report)
                    {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if onnx {
                let fixture = flag_value(&args, "--fixture-db")
                    .map(PathBuf::from)
                    .unwrap_or_else(crate::fixture::default_fixture_2k_path);
                let report = crate::bench::run_onnx_fixture_bench(&fixture)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert-target") {
                    if let Err(err) = crate::bench::assert_onnx_bench_target(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if scale {
                let full = args.iter().any(|a| a == "--full");
                let sizes: &[usize] = if full {
                    crate::scale_bench::SCALE_SIZES
                } else {
                    &[1_000]
                };
                let report = crate::scale_bench::run_scale_bench(sizes)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) = crate::scale_bench::assert_scale_bench_gate(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if ci {
                let report = crate::bench::run_ci_bench()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Err(err) = crate::bench::assert_bench_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            } else {
                eprintln!("Usage: agent-brain bench --ci");
                eprintln!(
                    "       agent-brain bench --beam              BEAM memory+routing harness"
                );
                eprintln!("       agent-brain bench --mcp [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --graphify [--full] [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --scale [--full] [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --supervisor [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --onnx [--fixture-db PATH] [--write PATH] [--assert-target]");
                std::process::exit(1);
            }
        }
        "fixture" => {
            let sub = args.get(1).map(String::as_str).unwrap_or("");
            match sub {
                "build" => {
                    let snapshot = flag_value(&args, "--snapshot")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::skills_sh::default_snapshot_path);
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::fixture::default_fixture_2k_path);
                    let index_size = flag_value(&args, "--index-size")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(crate::skills_sh::SKILLS_SH_SIMULATED_INDEX);
                    let allow_fillers = args.iter().any(|a| a == "--allow-fillers");
                    let report = crate::skills_sh::build_fixture_db(
                        &snapshot,
                        index_size,
                        &write_path,
                        allow_fillers,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    eprintln!(
                        "Built fixture DB ({} skills, {} snapshot) → {}",
                        report.index_size,
                        report.snapshot_skills,
                        write_path.display()
                    );
                }
                "verify" => {
                    let path = flag_value(&args, "--db")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::fixture::default_fixture_2k_path);
                    let meta = crate::fixture::read_fixture_meta(&path)?;
                    let breakdown = {
                        let dir = tempfile::tempdir()?;
                        let config = Config::isolated(dir.path().to_path_buf());
                        config.ensure_dirs()?;
                        std::fs::copy(&path, &config.db_path)?;
                        let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
                        crate::fixture::fixture_db_breakdown(&store)?
                    };
                    #[derive(serde::Serialize)]
                    struct VerifyOut {
                        meta: crate::fixture::FixtureDbMeta,
                        breakdown: crate::fixture::FixtureDbBreakdown,
                    }
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&VerifyOut { meta, breakdown })?
                    );
                }
                _ => {
                    eprintln!("Usage: agent-brain fixture build [--snapshot PATH] [--index-size N] [--write PATH] [--allow-fillers]");
                    eprintln!("       agent-brain fixture verify [--db PATH]");
                    std::process::exit(1);
                }
            }
        }
        "skills-sh" => {
            let sub = args.get(1).map(String::as_str).unwrap_or("");
            match sub {
                "sync" => {
                    let manifest_path = flag_value(&args, "--manifest")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::skills_sh::default_manifest_path);
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::skills_sh::default_snapshot_path);
                    let target = flag_value(&args, "--target")
                        .or_else(|| flag_value(&args, "--max"))
                        .and_then(|v| v.parse().ok());
                    let required_only = args.iter().any(|a| a == "--required-only");
                    let merge = args.iter().any(|a| a == "--merge");
                    let mut manifest = crate::skills_sh::load_manifest(&manifest_path)?;
                    if required_only {
                        manifest.discovery_queries.clear();
                        manifest.max_skills = manifest.required_ids.len();
                    }
                    let delay_ms = flag_value(&args, "--delay-ms")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(if required_only { 20_000 } else { 400 });
                    let mut options =
                        crate::skills_sh::SyncOptions::from_manifest(&manifest, target, delay_ms);
                    if merge || write_path.exists() {
                        options.merge_path = Some(write_path.clone());
                    }
                    options.checkpoint_path = Some(write_path.clone());
                    let (snapshot, report) =
                        crate::skills_sh::sync_snapshot_with_options(&manifest, options)?;
                    crate::skills_sh::write_snapshot(&write_path, &snapshot)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    eprintln!(
                        "Synced {} skills ({} downloaded, {} metadata) → {}",
                        snapshot.skills.len(),
                        report.downloaded,
                        report.metadata_fallback,
                        write_path.display()
                    );
                }
                "retry-failed" => {
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::skills_sh::default_snapshot_path);
                    let delay_ms = flag_value(&args, "--delay-ms")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(200);
                    let max_retries = flag_value(&args, "--max").and_then(|v| v.parse().ok());
                    let download_attempts = flag_value(&args, "--download-attempts")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(2);
                    let (snapshot, report) = crate::skills_sh::retry_failed_downloads(
                        &write_path,
                        delay_ms,
                        download_attempts,
                        max_retries,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    eprintln!(
                        "Retry complete: {} upgraded, {} still metadata, {} total → {}",
                        report.upgraded,
                        report.still_metadata,
                        snapshot.skills.len(),
                        write_path.display()
                    );
                }
                "golden-probe" => {
                    let fixture = flag_value(&args, "--fixture-db")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::fixture::default_fixture_2k_path);
                    let snapshot = flag_value(&args, "--snapshot")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::skills_sh::default_snapshot_path);
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(crate::skills_sh::default_golden_path);
                    let target = flag_value(&args, "--target")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(25);
                    let golden = crate::skills_sh::probe_golden_cases(&fixture, &snapshot, target)?;
                    crate::skills_sh::write_golden(&write_path, &golden)?;
                    eprintln!(
                        "Probed {} golden cases (target {}) → {}",
                        golden.cases.len(),
                        target,
                        write_path.display()
                    );
                    println!("{}", serde_json::to_string_pretty(&golden)?);
                }
                _ => {
                    eprintln!("Usage: agent-brain skills-sh sync [--target N] [--merge] [--manifest PATH] [--write PATH] [--delay-ms N] [--required-only]");
                    eprintln!("       agent-brain skills-sh retry-failed [--write PATH] [--max N] [--delay-ms N]");
                    eprintln!("       agent-brain skills-sh golden-probe [--target N] [--fixture-db PATH] [--snapshot PATH] [--write PATH]");
                    std::process::exit(1);
                }
            }
        }
        "eval" => {
            let live = args.iter().any(|a| a == "--live");
            let skills_sh = args.iter().any(|a| a == "--skills-sh");
            if skills_sh {
                let seed_runtime = args.iter().any(|a| a == "--seed");
                let snapshot = flag_value(args, "--snapshot")
                    .map(PathBuf::from)
                    .unwrap_or_else(crate::skills_sh::default_snapshot_path);
                let golden = flag_value(args, "--golden")
                    .map(PathBuf::from)
                    .unwrap_or_else(crate::skills_sh::default_golden_path);
                let fixture_db: Option<PathBuf> = if seed_runtime {
                    None
                } else {
                    flag_value(args, "--fixture-db")
                        .map(PathBuf::from)
                        .or_else(|| {
                            let default = crate::fixture::default_fixture_2k_path();
                            default.exists().then_some(default)
                        })
                };
                let report = crate::skills_sh::run_skills_sh_eval(
                    &snapshot,
                    &golden,
                    fixture_db.as_deref(),
                )?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if let Err(err) = crate::skills_sh::assert_skills_sh_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            } else if !args.iter().any(|a| a == "--ci") {
                eprintln!("Usage: agent-brain eval --ci [--live]");
                eprintln!("       agent-brain eval --skills-sh [--fixture-db PATH] [--seed] [--write PATH]");
                eprintln!("  --ci            Run Recall@3 gate (default: isolated fixture DB)");
                eprintln!(
                    "  --live          Embed with the real model on an isolated DB (not for CI)"
                );
                eprintln!("  --skills-sh     Run skills.sh Recall@3 (~2000 index)");
                eprintln!("  --fixture-db    Committed benchmark DB (default: docs/benchmarks/fixture-2k.db)");
                eprintln!(
                    "  --seed          Seed index at runtime from snapshot instead of fixture DB"
                );
                std::process::exit(1);
            } else {
                let report = if live {
                    crate::eval::run_ci_eval_live()?
                } else {
                    crate::eval::run_ci_eval_isolated()?
                };
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Err(err) = crate::eval::assert_ci_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            }
        }
        "proofs" => {
            if !args.iter().any(|a| a == "--ci") {
                eprintln!("Usage: agent-brain proofs --ci [--write PATH]");
                std::process::exit(1);
            }
            let report = crate::proofs::run_ci_proofs()?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if let Some(path) = flag_value(&args, "--write") {
                crate::proofs::write_proof_report(PathBuf::from(path).as_path(), &report)?;
            }
            if let Ok(config) = Config::load() {
                let _ = persist_proof_snapshot(&config.home, &report);
            }
            if let Err(err) = crate::proofs::assert_ci_proofs(&report) {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        _ => anyhow::bail!("unknown bench extension command: {cmd}"),
    }
    Ok(())
}

fn persist_proof_snapshot(home: &Path, report: &crate::proofs::ProofReport) -> Result<()> {
    let dir = home.join("metrics");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("proof-latest.json");
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}
