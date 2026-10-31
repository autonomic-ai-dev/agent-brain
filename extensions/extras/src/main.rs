use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use agent_brain::config::Config;
use agent_brain::engine::Engine;
use anyhow::{bail, Context, Result};

mod dataset;
mod distill;
mod grpc;
mod operator_digest;
mod spine_ingest;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("dataset") => dataset(&args),
        Some("distill") => distill_cmd(&args),
        Some("digest") => digest(&args),
        Some("grpc") => serve_grpc(&args).await,
        _ => bail!("usage: agent-brain <dataset|digest|distill|grpc> ..."),
    }
}

fn dataset(args: &[String]) -> Result<()> {
    let config = Config::load()?;
    let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
    let sub = args.get(2).map(String::as_str).unwrap_or("help");
    match sub {
        "export" => {
            let min_confidence = flag_value(&args, "--min-confidence")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.8);
            let only_successful = !args.iter().any(|a| a == "--all-outcomes");
            let out = flag_value(&args, "--out")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("dataset.jsonl"));
            let entries = dataset::export_dataset(&store, min_confidence, only_successful)?;
            let file = std::fs::File::create(&out)?;
            let mut writer = std::io::BufWriter::new(file);
            for entry in &entries {
                let line = serde_json::to_string(entry)?;
                writeln!(writer, "{line}")?;
            }
            writer.flush()?;
            println!("Exported {} entries to {}", entries.len(), out.display());
        }
        "stats" => {
            let min_confidence = flag_value(&args, "--min-confidence")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.8);
            let entries = dataset::export_dataset(&store, min_confidence, true)?;
            let stats = dataset::compute_stats(&entries);
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        "ingest-spine" => {
            let only_successful = !args.iter().any(|a| a == "--all-outcomes");
            let out = flag_value(&args, "--out")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(spine_ingest::default_dataset_path);
            let executions = flag_value(&args, "--executions-dir").map(std::path::PathBuf::from);
            let report =
                spine_ingest::ingest_executions(executions.as_deref(), &out, only_successful)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        _ => {
            eprintln!("Usage: agent-brain dataset export [--out PATH] [--min-confidence N] [--all-outcomes]");
            eprintln!("       agent-brain dataset stats [--min-confidence N]");
            eprintln!("       agent-brain dataset ingest-spine [--out PATH] [--executions-dir DIR] [--all-outcomes]");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn distill_cmd(args: &[String]) -> Result<()> {
    let is_clusters = args.get(2).map(|s| s.as_str()) == Some("clusters");
    if is_clusters {
        let config = Config::load()?;
        config.ensure_dirs()?;
        let threshold = flag_value(&args, "--threshold")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.95);
        let dry_run = has_flag(&args, "--dry-run");
        let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
        let stats = distill::cluster_and_summarize(&store, threshold, dry_run)?;
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        let config = Config::load()?;
        config.ensure_dirs()?;
        let out = flag_value(&args, "--out")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("ARCHITECTURE.md"));
        let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
        let distilled = distill::distill(&store)?;
        distill::write_architecture_md(&distilled, &out)?;
        println!("Architecture written to {}", out.display());
    }
    Ok(())
}

async fn serve_grpc(args: &[String]) -> Result<()> {
    let sub = args.get(2).map(String::as_str).unwrap_or("serve");
    if sub != "serve" {
        bail!("usage: agent-brain grpc serve [--addr HOST:PORT]");
    }
    let addr = flag_value(args, "--addr").unwrap_or_else(|| "127.0.0.1:7842".to_string());
    let addr: SocketAddr = addr.parse().context("invalid --addr")?;
    let config = Config::load()?;
    let engine = Arc::new(Engine::new(config)?);
    if engine.config.bootstrap_background {
        engine.spawn_bootstrap(None);
    } else {
        let n = engine.bootstrap(None)?;
        let total = engine.store.count_indexed_items().unwrap_or(0);
        tracing::info!(
            "indexed {}",
            agent_brain::install::format_new_index_count(n, total)
        );
    }
    grpc::serve(engine, addr).await
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn digest(args: &[String]) -> Result<()> {
    if !args.iter().any(|a| a == "--weekly") {
        eprintln!("Usage: agent-brain digest --weekly");
        std::process::exit(1);
    }
    let config = Config::load()?;
    let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
    let digest = crate::operator_digest::weekly_digest(&store, 7)?;
    println!("{}", crate::operator_digest::format_weekly_digest(&digest));
    Ok(())
}
