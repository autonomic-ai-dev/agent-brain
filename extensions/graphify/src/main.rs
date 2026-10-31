use std::path::PathBuf;
use std::sync::Arc;

use agent_brain::config::Config;
use agent_brain::engine::Engine;
use agent_brain::graphify::{run_cli, GraphifyCli};
use anyhow::{bail, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("graphify") {
        bail!("usage: agent-brain graphify <enable|disable|status|ingest|run|query> ...");
    }
    let config = Config::load()?;
    let engine = Arc::new(Engine::new(config)?);
    let sub = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("status")
        .to_string();
    let cli = GraphifyCli {
        sub: sub.clone(),
        repo: flag_value(&args, "--repo").map(PathBuf::from),
        trigger: flag_value(&args, "--trigger"),
        mode: flag_value(&args, "--mode"),
        job_id: flag_value(&args, "--id"),
        question: flag_value(&args, "--question")
            .or_else(|| args.get(2).filter(|_| sub == "query").cloned()),
        budget: flag_value(&args, "--budget")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1500),
    };
    run_cli(&engine, cli)
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}
