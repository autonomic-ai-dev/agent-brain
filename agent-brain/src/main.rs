use std::sync::Arc;

use agent_brain::{
    auto_update, config::Config, doctor, engine::Engine, install, mcp, packages, serve_meta,
    settings,
};
use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("agent_brain=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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
            if let Err(err) = serve_meta::write_current(&engine.config.home) {
                tracing::warn!(error = %err, "write serve_meta.json failed");
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
            let reload = args.iter().any(|a| a == "--reload");
            let print_only = args.iter().any(|a| a == "--print-only");
            install::run(global || reload, print_only, reload)?;
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
            let mcp_only = args.iter().any(|a| a == "--mcp-only");
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
                mcp_only,
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
        "version" => {
            println!("agent-brain {}", env!("CARGO_PKG_VERSION"));
        }
        "briefing" => {
            let config = Config::load()?;
            let path = config.home.join("logs").join("last-route.md");
            if path.exists() {
                print!("{}", std::fs::read_to_string(&path)?);
            } else {
                eprintln!("No route briefing yet. Run a turn with route_task first.");
                eprintln!("Briefings are written to {}", path.display());
                std::process::exit(1);
            }
        }
        "export" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let dest = args.get(2).map(std::path::PathBuf::from);
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let path = agent_brain::sync::export_bundle(&store, &config.home, dest.as_deref())?;
            println!("Exported sync bundle to {}", path.display());
        }
        "import" => {
            let bundle = args
                .get(2)
                .context("usage: agent-brain import <bundle-dir> [--policy newer_wins|keep_local|keep_remote]")?;
            let policy = flag_value(&args, "--policy")
                .and_then(|s| agent_brain::sync::MergePolicy::parse(&s))
                .unwrap_or(agent_brain::sync::MergePolicy::NewerWins);
            let config = Config::load()?;
            config.ensure_dirs()?;
            let engine = Arc::new(Engine::new(config)?);
            let report = engine.import_bundle_queued(
                std::path::Path::new(bundle),
                policy,
                agent_brain::sync::SyncSource::ManualImport,
            )?;
            engine.bootstrap(None)?;
            println!(
                "Imported {} facts (deduped {}, conflicts {}, skipped {})",
                report.imported, report.deduplicated, report.conflicts_resolved, report.skipped
            );
        }
        "sync" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let brain_settings = settings::AgentBrainSettings::load(&config.home);
            let sub = args.get(2).map(String::as_str).unwrap_or("help");
            match sub {
                "status" => {
                    let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
                    let status = agent_brain::sync::sync_status(
                        &config.home,
                        &brain_settings.sync.git,
                        &brain_settings.sync.cloud,
                        &store,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                "restore" => {
                    let conflict_id = args
                        .get(3)
                        .context("usage: agent-brain sync restore <conflict-id>")?;
                    let engine = Arc::new(Engine::new(config)?);
                    let fact_id = agent_brain::sync::restore_conflict(
                        &engine.store,
                        &engine.embedder,
                        conflict_id,
                    )?;
                    engine.store.bump_index_version()?;
                    engine.bootstrap(None)?;
                    println!("Restored fact {fact_id} from conflict {conflict_id}");
                }
                "cloud" => {
                    let cloud_cmd = args.get(3).map(String::as_str).unwrap_or("help");
                    match cloud_cmd {
                        "push" => {
                            let store =
                                agent_brain::db::store::BrainStore::open(&config.db_path)?;
                            agent_brain::sync::cloud_push(
                                &store,
                                &config.home,
                                &brain_settings.sync.cloud,
                            )?;
                            println!(
                                "Pushed encrypted bundle to {} ({})",
                                brain_settings.sync.cloud.bucket, brain_settings.sync.cloud.key
                            );
                        }
                        "pull" => {
                            let engine = Arc::new(Engine::new(config)?);
                            let report = agent_brain::sync::cloud_pull(
                                &engine,
                                &brain_settings.sync.cloud,
                            )?;
                            engine.bootstrap(None)?;
                            println!(
                                "Pulled and imported {} facts (deduped {}, conflicts {}, skipped {})",
                                report.import.imported,
                                report.import.deduplicated,
                                report.import.conflicts_resolved,
                                report.import.skipped
                            );
                        }
                        _ => {
                            eprintln!("Usage: agent-brain sync cloud push|pull");
                            std::process::exit(1);
                        }
                    }
                }
                "git" => {
                    let git_cmd = args.get(3).map(String::as_str).unwrap_or("help");
                    match git_cmd {
                        "init" => {
                            let remote = flag_value(&args, "--remote")
                                .or_else(|| {
                                    let r = brain_settings.sync.git.remote.clone();
                                    if r.is_empty() { None } else { Some(r) }
                                });
                            let branch = brain_settings.sync.git.branch.clone();
                            let root = agent_brain::sync::init_git_repo(
                                &config.home,
                                remote.as_deref(),
                                &branch,
                            )?;
                            println!("Initialized git sync repo at {}", root.display());
                            if remote.is_none() {
                                eprintln!(
                                    "Tip: set sync.git.remote in config.yaml, then run sync git push"
                                );
                            }
                        }
                        "clone" => {
                            let remote = flag_value(&args, "--remote")
                                .or_else(|| {
                                    let r = brain_settings.sync.git.remote.clone();
                                    if r.is_empty() { None } else { Some(r) }
                                })
                                .context("usage: agent-brain sync git clone [--remote URL]")?;
                            let branch = brain_settings.sync.git.branch.clone();
                            let root = agent_brain::sync::git_clone(
                                &config.home,
                                &remote,
                                &branch,
                            )?;
                            println!("Cloned sync repo to {}", root.display());
                            eprintln!("Run: agent-brain sync git pull");
                        }
                        "push" => {
                            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
                            agent_brain::sync::git_push(&store, &config.home, &brain_settings.sync.git)?;
                            println!("Pushed memory bundle to origin/{}", brain_settings.sync.git.branch);
                        }
                        "pull" => {
                            let engine = Arc::new(Engine::new(config)?);
                            let report = agent_brain::sync::git_pull(
                                &engine,
                                &brain_settings.sync.git,
                            )?;
                            engine.bootstrap(None)?;
                            println!(
                                "Pulled and imported {} facts (deduped {}, conflicts {}, skipped {})",
                                report.imported,
                                report.deduplicated,
                                report.conflicts_resolved,
                                report.skipped
                            );
                        }
                        "status" => {
                            let status = agent_brain::sync::git_status(
                                &config.home,
                                &brain_settings.sync.git,
                            )?;
                            println!("{}", serde_json::to_string_pretty(&status)?);
                        }
                        _ => {
                            eprintln!("Usage: agent-brain sync git init [--remote URL]");
                            eprintln!("       agent-brain sync git clone [--remote URL]");
                            eprintln!("       agent-brain sync git push");
                            eprintln!("       agent-brain sync git pull");
                            eprintln!("       agent-brain sync git status");
                            std::process::exit(1);
                        }
                    }
                }
                _ => {
                    eprintln!("Usage: agent-brain sync status");
                    eprintln!("       agent-brain sync restore <conflict-id>");
                    eprintln!("       agent-brain sync git init|clone|push|pull|status");
                    eprintln!("       agent-brain sync cloud push|pull");
                    std::process::exit(1);
                }
            }
        }
        "doctor" => {
            let fix = args.iter().any(|a| a == "--fix");
            doctor::run(fix)?;
        }
        "secrets" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let sub = args.get(2).map(String::as_str).unwrap_or("status");
            match sub {
                "status" => {
                    let status = agent_brain::secrets::secrets_status(&store)?;
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                "setup" => agent_brain::secrets::setup_missing_interactive(&store)?,
                "add" => {
                    let name = args
                        .get(3)
                        .context("usage: agent-brain secrets add <NAME> --used-by <target>")?;
                    let used_by = flag_value(&args, "--used-by")
                        .unwrap_or_else(|| "upstream_mcp".into());
                    store.upsert_secret_ref(name, &used_by)?;
                    println!("Registered secret ref {name} (used by {used_by})");
                }
                _ => {
                    eprintln!("Usage: agent-brain secrets status");
                    eprintln!("       agent-brain secrets setup");
                    eprintln!("       agent-brain secrets add <NAME> --used-by <target>");
                    std::process::exit(1);
                }
            }
        }
        "inspect" => {
            let config = Config::load()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let sub = args.get(2).map(String::as_str).unwrap_or("log");
            match sub {
                "log" => {
                    let last = args.iter().any(|a| a == "--last");
                    if last {
                        if let Some(row) = store.latest_retrieval_log()? {
                            println!("{}", agent_brain::observability::format_inspect_log(&row));
                            println!("items: {}", row.items_json);
                        } else {
                            eprintln!("No retrieval logs yet.");
                            std::process::exit(1);
                        }
                    } else {
                        let limit = flag_value(&args, "--limit")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(10);
                        for row in store.list_retrieval_logs(limit)? {
                            println!("{}", agent_brain::observability::format_inspect_log(&row));
                        }
                    }
                }
                "fact" => {
                    let id = args
                        .get(3)
                        .context("usage: agent-brain inspect fact <id>")?;
                    match store.get_fact(id)? {
                        Some(f) => println!("{}", serde_json::to_string_pretty(&f)?),
                        None => {
                            eprintln!("Fact not found: {id}");
                            std::process::exit(1);
                        }
                    }
                }
                "conflicts" => {
                    let limit = flag_value(&args, "--limit")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(20);
                    for row in store.list_conflicts(limit)? {
                        println!("{}", serde_json::to_string(&row)?);
                    }
                }
                _ => {
                    eprintln!("Unknown inspect subcommand: {sub}");
                    eprintln!("Usage: agent-brain inspect log [--last] [--limit N]");
                    eprintln!("       agent-brain inspect fact <id>");
                    eprintln!("       agent-brain inspect conflicts [--limit N]");
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
  agent-brain install [--global] [--reload]     Write Cursor MCP config for this binary
  agent-brain update [--force] [--mcp-only]     Run auto-update (MCP checks GitHub tag; packages use 24h interval unless --force)
  agent-brain config init                     Write ~/.agent_brain/config.yaml defaults
  agent-brain config show                     Print active config file
  agent-brain version                         Print installed version
  agent-brain briefing                        Print last human-readable route summary
  agent-brain export [dir]                    Export sync bundle (manifest + facts.jsonl)
  agent-brain import <dir> [--policy POLICY]  Import sync bundle (newer_wins default)
  agent-brain sync status                     Git sync + recent conflicts summary
  agent-brain sync restore <conflict-id>      Re-promote a fact from conflict_log
  agent-brain sync git init [--remote URL]    Init ~/.agent_brain/sync git repo (S2)
  agent-brain sync git clone [--remote URL]   Clone sync repo on a second machine
  agent-brain sync git push                   Export bundle, commit, push to origin
  agent-brain sync git pull                   Pull from origin and import bundle
  agent-brain sync git status                 Show git sync repo state
  agent-brain sync cloud push|pull            Encrypted cloud sync (S3 / local provider)
  agent-brain secrets status|setup|add        Keychain secret refs for upstream MCP
  agent-brain doctor                          Check MCP install, binary, hooks, codesign
  agent-brain doctor --fix                    Re-sign binary (macOS), align mcp.json, refresh hooks
  agent-brain inspect log [--last]            List retrieval logs (what route_task returned)
  agent-brain inspect fact <id>               Show a stored memory fact
  agent-brain inspect conflicts [--limit N]   Show topic supersession conflict log
  agent-brain --version                       Same as version (prints version only)

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
