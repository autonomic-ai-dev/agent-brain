use std::sync::Arc;

use agent_brain::{auto_update, config::Config, engine::Engine, install, mcp, packages, settings};
use anyhow::{Context, Result};
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
            let brain_settings = settings::AgentBrainSettings::load(&config.home);
            let engine = Arc::new(Engine::new(config)?);
            if engine.config.bootstrap_background {
                tracing::info!(target: "agent_brain::bootstrap", "starting MCP; bootstrap runs in background");
                engine.spawn_bootstrap(None);
            } else {
                let n = engine.bootstrap(None)?;
                tracing::info!("indexed {n} items");
            }
            if brain_settings.auto_update.enabled {
                engine.spawn_auto_update();
            }
            mcp::run_stdio(engine).await?;
        }
        "index" => {
            let mut config = Config::load()?;
            config.bootstrap_background = false;
            config.session_ingest_background = false;
            config.bootstrap_interval_secs = 0;
            let engine = Arc::new(Engine::new(config)?);
            let n = engine.bootstrap(None)?;
            println!("Indexed {n} items");
        }
        "add" => {
            let source = args.get(2).context("missing package source (owner/repo or GitHub URL)")?;
            let git_ref = flag_value(&args, "--ref");
            let skip_index = args.iter().any(|a| a == "--no-index");
            let config = Config::load()?;
            let record = packages::add_package(&config, source, git_ref.as_deref())?;
            println!(
                "Installed package '{}' from {} ({})",
                record.name,
                record.source,
                record.commit.unwrap_or_else(|| "unknown".into())
            );
            if !skip_index {
                let mut config = config;
                config.bootstrap_background = false;
                config.session_ingest_background = false;
                let engine = Arc::new(Engine::new(config)?);
                let n = engine.bootstrap(None)?;
                println!("Indexed {n} items");
            }
        }
        "package" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("list");
            let config = Config::load()?;
            match sub {
                "list" => {
                    for pkg in packages::list_packages(&config)? {
                        println!(
                            "{}  {}  ref={}  commit={}  path={}",
                            pkg.name,
                            pkg.source,
                            pkg.git_ref,
                            pkg.commit.unwrap_or_else(|| "-".into()),
                            pkg.install_path
                        );
                    }
                }
                "remove" => {
                    let name = args
                        .get(3)
                        .context("usage: agent-brain package remove <name>")?;
                    let purged = packages::remove_package(&config, name)?;
                    println!("Removed package '{name}' (purged {purged} indexed items)");
                }
                "update" => {
                    let name = args.get(3).map(String::as_str);
                    let updated = packages::update_packages(&config, name)?;
                    for pkg in updated {
                        println!("Updated {} ({})", pkg.name, pkg.commit.unwrap_or_default());
                    }
                    let mut config = config;
                    config.bootstrap_background = false;
                    config.session_ingest_background = false;
                    let engine = Arc::new(Engine::new(config)?);
                    let n = engine.bootstrap(None)?;
                    println!("Indexed {n} items");
                }
                _ => {
                    eprintln!("Unknown package subcommand: {sub}");
                    print_usage();
                    std::process::exit(1);
                }
            }
        }
        "install" => {
            let global = args.iter().any(|a| a == "--global");
            let print_only = args.iter().any(|a| a == "--print-only");
            install::run(global, print_only)?;
            if global && !print_only {
                let config = Config::load()?;
                if settings::config_path_optional(&config.home).is_none() {
                    let path = settings::AgentBrainSettings::save_default(&config.home)?;
                    println!("Wrote default auto-update config at {}", path.display());
                }
            }
        }
        "update" => {
            let force = args.iter().any(|a| a == "--force" || a == "-f");
            let config = Config::load()?;
            let brain_settings = settings::AgentBrainSettings::load(&config.home);
            if !brain_settings.auto_update.enabled {
                eprintln!("Auto-update is disabled. Enable it in ~/.agent_brain/config.yaml or set AGENT_BRAIN_AUTO_UPDATE=1");
                std::process::exit(1);
            }
            let mut run_config = config.clone();
            run_config.bootstrap_background = false;
            run_config.session_ingest_background = false;
            let engine = Arc::new(Engine::new(run_config)?);
            let report = auto_update::run(
                &engine,
                &brain_settings,
                force,
                auto_update::AutoUpdateRunOptions::cli(),
            )?;
            if report.packages_updated == 0 && !report.mcp_updated {
                println!("Nothing to update.");
            } else {
                if report.mcp_updated {
                    println!(
                        "Updated MCP binary to v{}.",
                        report.mcp_version.unwrap_or_default()
                    );
                    println!(
                        "Toggle agent-brain off/on in Cursor Settings → MCP to load it (or wait for background auto-update to restart serve)."
                    );
                }
                if report.packages_updated > 0 {
                    println!("Updated {} package(s)", report.packages_updated);
                }
                if report.reindexed {
                    println!("Reindexed after update");
                }
            }
        }
        "config" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("show");
            let config = Config::load()?;
            match sub {
                "init" => {
                    let path = settings::AgentBrainSettings::save_default(&config.home)?;
                    println!("Wrote {}", path.display());
                }
                "show" => {
                    match settings::config_path_optional(&config.home) {
                        Some(path) => {
                            let raw = std::fs::read_to_string(&path)?;
                            print!("{raw}");
                        }
                        None => {
                            eprintln!("No config file. Run: agent-brain config init");
                            std::process::exit(1);
                        }
                    }
                }
                _ => {
                    eprintln!("Unknown config subcommand: {sub}");
                    print_usage();
                    std::process::exit(1);
                }
            }
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

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}

fn print_usage() {
    eprintln!(
        r#"agent-brain — local MCP router for agents, skills, rules, and memory

Usage:
  agent-brain serve                         Start MCP server (stdio)
  agent-brain index                         Reindex local agents/skills/rules/memory
  agent-brain add <owner/repo|url>          Install a GitHub package and index it
  agent-brain add affaan-m/ecc --ref main   Install with explicit git ref
  agent-brain package list                  List installed packages
  agent-brain package update [name]         Update one or all packages
  agent-brain package remove <name>         Remove an installed package
  agent-brain install [--global]              Write Cursor MCP config for this binary
  agent-brain update [--force]                Run configured package/MCP auto-update now
  agent-brain config init                     Write ~/.agent_brain/config.yaml defaults
  agent-brain config show                     Print active config file

Examples:
  agent-brain add https://github.com/affaan-m/ecc
  agent-brain add affaan-m/ecc
  agent-brain package update ecc

Install on another machine:
  curl -fsSL https://raw.githubusercontent.com/aeswibon/agent-brain/master/scripts/install.sh | bash -s -- --global
  agent-brain add affaan-m/ecc

Cursor starts MCP automatically — you do not run 'serve' manually.
See docs/USAGE.md for the full guide.
"#
    );
}
