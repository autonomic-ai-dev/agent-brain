use std::sync::Arc;

use agent_brain::{config::Config, engine::Engine, install, mcp};
use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("agent_brain=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("serve");

    match cmd {
        "serve" => {
            let config = Config::load()?;
            let engine = Arc::new(Engine::new(config)?);
            let n = engine.bootstrap(None)?;
            tracing::info!("indexed {n} items");
            mcp::run_stdio(engine).await?;
        }
        "index" => {
            let config = Config::load()?;
            let engine = Engine::new(config)?;
            let n = engine.bootstrap(None)?;
            println!("Indexed {n} items");
        }
        "install" => {
            let global = args.iter().any(|a| a == "--global");
            let print_only = args.iter().any(|a| a == "--print-only");
            install::run(global, print_only)?;
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        _ => {
            eprintln!("Unknown command: {cmd}");
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_usage() {
    eprintln!(
        r#"agent-brain — local MCP router for agents, skills, rules, and memory

Usage:
  agent-brain serve              Start MCP server (stdio)
  agent-brain index              Reindex local agents/skills/rules/memory
  agent-brain install            Write Cursor MCP config for this binary
  agent-brain install --global   Write ~/.cursor/mcp.json
  agent-brain install --print-only   Print MCP JSON only

Install on another machine:
  curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/main/scripts/install.sh | bash -s -- --global

Or with Rust:
  cargo install --git https://github.com/aeswibon/agent-brain agent-brain
  agent-brain install --global
"#
    );
}
