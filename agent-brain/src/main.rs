use std::net::SocketAddr;
use std::sync::Arc;

use agent_brain::{
    auto_update, config::Config, doctor, engine::Engine, host_install, index, install, mcp,
    mcp_cli, packages, registry_sync, serve_meta, settings,
};
use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

mod cli_help;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("agent_brain=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let args = agent_body_core::cli::strip_progress_argv(std::env::args().collect());
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(target) = cli_help::help_target(&args) {
        match target {
            None => cli_help::print_root(),
            Some(cmd) => {
                if !cli_help::print_command(cmd) {
                    eprintln!("Unknown command: {cmd}");
                    cli_help::print_root();
                    std::process::exit(1);
                }
            }
        }
        return Ok(());
    }

    let cmd = args.get(1).map(String::as_str).unwrap_or("serve");

    #[cfg(feature = "extras")]
    if agent_brain::cli_extras::is_extras_command(cmd) {
        return agent_brain::cli_extras::dispatch(cmd, &args);
    }
    #[cfg(not(feature = "extras"))]
    if matches!(
        cmd,
        "dataset"
            | "digest"
            | "eval"
            | "fixture"
            | "skills-sh"
            | "bench"
            | "proofs"
            | "distill"
            | "graphify"
            | "grpc"
    ) {
        anyhow::bail!(
            "command `{cmd}` requires a build with `--features extras`\n             rebuild: cargo install --path agent-brain --features extras"
        );
    }

    match cmd {
        "serve" => {
            let config = Config::load()?;
            let daemon = args.iter().any(|a| a == "--daemon");
            let health_addr = flag_value(&args, "--health-addr")
                .and_then(|s| s.parse::<SocketAddr>().ok());
            let socket_path = config.home.join("serve.sock");
            if daemon {
                let _serve_lock = agent_brain::serve_lock::acquire(&config.home)?;
                let brain_settings = settings::AgentBrainSettings::load(&config.home);
                let engine = Arc::new(Engine::new(config)?);
                if engine.config.bootstrap_background {
                    tracing::info!(target: "agent_brain::bootstrap", "starting MCP; bootstrap runs in background");
                    engine.spawn_bootstrap(None);
                } else {
                    let n = engine.bootstrap(None)?;
                    let total = engine.store.count_indexed_items().unwrap_or(0);
                    tracing::info!(
                        "indexed {}",
                        install::format_new_index_count(n, total)
                    );
                }
                if brain_settings.auto_update.enabled {
                    engine.spawn_auto_update();
                }
                if let Err(err) = serve_meta::write_current(&engine.config.home) {
                    tracing::warn!(error = %err, "write serve_meta.json failed");
                }
                if let Err(e) = agent_brain::autowire::auto_wire(
                    &std::env::current_exe()?.display().to_string(),
                    env!("CARGO_PKG_VERSION"),
                ) {
                    tracing::warn!("Auto-wire failed: {}", e);
                }
                mcp::run_daemon(engine, &socket_path, health_addr).await?;
            } else {
                // Proxy mode: no exclusive lock. Bridges stdio to the supervised daemon's
                // Unix socket; reconnects if the daemon restarts.
                mcp::run_proxy(&socket_path).await?;
            }
        }
        #[cfg(feature = "extras")]
        "grpc" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("serve");
            if sub != "serve" {
                anyhow::bail!("usage: agent-brain grpc serve [--addr HOST:PORT]");
            }
            let addr = flag_value(&args, "--addr").unwrap_or_else(|| "127.0.0.1:7842".to_string());
            let addr: std::net::SocketAddr = addr.parse().context("invalid --addr")?;
            let config = Config::load()?;
            let engine = Arc::new(Engine::new(config)?);
            if engine.config.bootstrap_background {
                engine.spawn_bootstrap(None);
            } else {
                let n = engine.bootstrap(None)?;
                let total = engine.store.count_indexed_items().unwrap_or(0);
                tracing::info!(
                    "indexed {}",
                    install::format_new_index_count(n, total)
                );
            }
            agent_brain::grpc::serve(engine, addr).await?;
        }
        "index" => {
            let changed_only = args.iter().any(|a| a == "--changed-only" || a == "-c");
            #[cfg(feature = "extras")]
            let no_ast = args.iter().any(|a| a == "--no-ast");
            #[cfg(not(feature = "extras"))]
            let no_ast = true;
            let mut config = Config::load()?;
            config.bootstrap_background = false;
            config.session_ingest_background = false;
            config.bootstrap_interval_secs = 0;
            let engine = Arc::new(Engine::new(config)?);
            let n = index::sync_index_opts(
                &engine.store,
                &engine.config,
                &engine.embedder,
                None,
                changed_only,
                !no_ast,
            )?;
            let total = engine.store.count_indexed_items()?;
            println!(
                "Indexed {}",
                install::format_new_index_count(n, total)
            );
        }
        "add" => {
            handle_package_add(&args, 2)?;
        }
        "registry" => match args.get(2).map(String::as_str).unwrap_or("list") {
            "list" => {
                let kind = args.get(3).map(String::as_str);
                let show_skills = kind.is_none()
                    || kind == Some("skill_package")
                    || kind == Some("skill")
                    || kind == Some("skills");
                let show_utils = kind == Some("utility") || kind == Some("utilities");
                let show_workflows = kind == Some("workflow") || kind == Some("workflows");

                if show_skills {
                    println!("skill_packages:");
                    for entry in packages::list_aliases()? {
                        let source = if let Some(bundle) = &entry.bundle {
                            format!("bundle:{bundle}")
                        } else {
                            entry.packages.join(", ")
                        };
                        println!("  @{:<14}  {}  [{}]", entry.alias, entry.description, source);
                    }
                }
                if show_utils {
                    if show_skills {
                        println!();
                    }
                    println!("utilities:");
                    for util in packages::list_utilities()? {
                        let invoke = util.invoke.join("; ");
                        println!(
                            "  {:<14}  ({})  {}  [{}]",
                            util.name, util.alias, util.description, invoke
                        );
                    }
                }
                if show_workflows {
                    if show_skills || show_utils {
                        println!();
                    }
                    println!("workflows:");
                    for wf in packages::list_workflows()? {
                        println!(
                            "  @{:<14}  {}  [{}]  {}",
                            wf.alias, wf.description, wf.organ, wf.invoke
                        );
                    }
                }
                if !show_skills && !show_utils && !show_workflows {
                    eprintln!("Unknown kind '{kind:?}'. Use skill_package, utility, or workflow.");
                    std::process::exit(1);
                }
            }
            "sync" => {
                let local_only = args.iter().any(|a| a == "--local");
                let config = Config::load()?;
                let brain_settings = settings::AgentBrainSettings::load(&config.home);
                let n = if local_only {
                    registry_sync::seed_embedded(&config.home)?
                } else {
                    registry_sync::sync(&config.home, &brain_settings.registry)?
                };
                println!(
                    "Synced {n} registry files to {}",
                    registry_sync::cache_dir(&config.home).display()
                );
            }
            other => {
                eprintln!(
                    "Usage: agent-brain registry list [--kind skill_package|utility|workflow]"
                );
                eprintln!("       agent-brain registry sync [--local]");
                if !other.is_empty() {
                    std::process::exit(1);
                }
            }
        },
        "package" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("list");
            let config = Config::load()?;
            match sub {
                "add" => {
                    handle_package_add(&args, 3)?;
                }
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
                    let _ = cli_help::print_command("package");
                    std::process::exit(1);
                }
            }
        }
        "mcp" => {
            mcp_cli::handle_mcp_command(&args).await?;
        }
        "mode" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("paths");
            match sub {
                "show" => println!("{}", host_install::AGENT_BRAIN_MODE_SNIPPET),
                "install" => {
                    let global = args.iter().any(|a| a == "--global");
                    host_install::install_agent_brain_modes(global, false)?;
                }
                "paths" => host_install::print_agent_brain_mode_paths(),
                _ => {
                    eprintln!("Usage: agent-brain mode [paths|show|install [--global]]");
                    std::process::exit(1);
                }
            }
        }
        "install" => {
            let target = host_install::HostTarget::from_args(&args);
            let reload = args.iter().any(|a| a == "--reload");
            let print_only = args.iter().any(|a| a == "--print-only");
            let global = args.iter().any(|a| a == "--global");
            let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
            let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
            install::run(target, print_only, reload, verbose, quiet)?;
            if (global || matches!(target, host_install::HostTarget::All)) && !print_only {
                let user_scope = global || matches!(target, host_install::HostTarget::All);
                host_install::install_agent_brain_modes(user_scope, !verbose)?;
                let config = Config::load()?;
                if settings::config_path_optional(&config.home).is_none() {
                    let path = settings::AgentBrainSettings::save_default(&config.home)?;
                    if verbose {
                        println!("Wrote default auto-update config at {}", path.display());
                    }
                }
            }
        }
        "update" => {
            let force = args.iter().any(|a| a == "--force" || a == "-f");
            let mcp_only = args.iter().any(|a| a == "--mcp-only");
            let config = Config::load()?;
            let brain_settings = settings::AgentBrainSettings::load(&config.home);
            if !brain_settings.auto_update.enabled {
                eprintln!("Auto-update is disabled. Enable it in ~/.autonomic/config.toml [brain] or [update.brain], or set AGENT_BRAIN_AUTO_UPDATE=1");
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
                "show" => match settings::config_path_optional(&config.home) {
                    Some(path) => {
                        let raw = std::fs::read_to_string(&path)?;
                        print!("{raw}");
                    }
                    None => {
                        eprintln!("No config file. Run: agent-brain config init");
                        std::process::exit(1);
                    }
                },
                _ => {
                    eprintln!("Unknown config subcommand: {sub}");
                    let _ = cli_help::print_command("config");
                    std::process::exit(1);
                }
            }
        }
        "version" => {
            println!("agent-brain {}", env!("CARGO_PKG_VERSION"));
        }
        "briefing" => {
            let config = Config::load()?;
            let path = config.logs_dir.join("last-route.md");
            if path.exists() {
                print!("{}", std::fs::read_to_string(&path)?);
            } else {
                eprintln!("No route briefing yet. Run a turn with route_task first.");
                eprintln!("Briefings are written to {}", path.display());
                std::process::exit(1);
            }
            if let Some(suggestion) =
                agent_brain::route_briefing::read_anti_pattern_suggestion(&config.home)
            {
                println!(
                    "\n---\nSuggested store_memory (from hook):\n{}\n",
                    serde_json::to_string_pretty(&suggestion)?
                );
            }
        }
        "learn" => {
            let config = Config::load()?;
            let engine = Arc::new(Engine::new(config)?);
            let sub = args.get(2).map(String::as_str).unwrap_or("");
            match sub {
                "url" => {
                    let url = args.get(3).context(
                        "usage: agent-brain learn url <https-url> [--topic NAME] [--dry-run]",
                    )?;
                    let topic = flag_value(&args, "--topic");
                    let dry_run = args.iter().any(|a| a == "--dry-run");
                    let report =
                        agent_brain::docs::learn_from_url(&engine, url, topic.as_deref(), dry_run)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                "allowlist" | "domains" => {
                    let settings = settings::AgentBrainSettings::load(&engine.config.home);
                    for domain in &settings.docs.allowed_domains {
                        println!("{domain}");
                    }
                }
                _ => {
                    eprintln!(
                        "Usage: agent-brain learn url <https-url> [--topic NAME] [--dry-run]"
                    );
                    eprintln!("       agent-brain learn allowlist");
                    std::process::exit(1);
                }
            }
        }
        #[cfg(feature = "extras")]
        "graphify" => {
            let config = Config::load()?;
            let engine = Arc::new(Engine::new(config)?);
            let sub = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("status")
                .to_string();
            let cli = agent_brain::graphify::GraphifyCli {
                sub: sub.clone(),
                repo: flag_value(&args, "--repo").map(std::path::PathBuf::from),
                trigger: flag_value(&args, "--trigger"),
                mode: flag_value(&args, "--mode"),
                job_id: flag_value(&args, "--id"),
                question: flag_value(&args, "--question").or_else(|| {
                    args.get(3)
                        .filter(|_| matches!(sub.as_str(), "query"))
                        .cloned()
                }),
                budget: flag_value(&args, "--budget")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1500),
            };
            agent_brain::graphify::run_cli(&engine, cli)?;
        }
        "suggest-memory" => {
            let config = Config::load()?;
            let sub = args.get(2).map(String::as_str).unwrap_or("show");
            match sub {
                "approve" => {
                    let engine = Arc::new(Engine::new(config)?);
                    let report = agent_brain::suggest_memory::approve_pending(&engine)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    if report.stored {
                        let n = engine.bootstrap(None)?;
                        println!("Stored negative memory (reindexed {n} items)");
                    } else if report.deduplicated {
                        println!("Already stored (deduplicated)");
                    }
                }
                "reject" => {
                    let config = Config::load()?;
                    agent_brain::suggest_memory::reject_pending(&config.home)?;
                    println!("Dismissed pending memory suggestion");
                }
                "show" | _ => match agent_brain::route_briefing::read_pending_memory_suggestion(
                    &Config::load()?.home,
                ) {
                    Some(suggestion) => {
                        println!("{}", serde_json::to_string_pretty(&suggestion)?);
                        println!("\nApprove: agent-brain suggest-memory approve");
                    }
                    None => {
                        eprintln!("No memory suggestion pending.");
                        std::process::exit(1);
                    }
                },
            }
        }


        "stats" => {
            let days = flag_value(&args, "--days")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(7);
            let json = args.iter().any(|a| a == "--json");
            let config = Config::load()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let snapshot = agent_brain::stats::collect(&store, &config, days)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                print!("{}", agent_brain::stats::format_text(&snapshot));
            }
        }

        "dashboard" => {
            let days = flag_value(&args, "--days")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(7);
            let open = args.iter().any(|a| a == "--open");
            let config = Config::load()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let snapshot = agent_brain::stats::collect(&store, &config, days)?;
            let path = agent_brain::dashboard::write_dashboard_html(&config.home, &snapshot)?;
            println!("{}", agent_brain::stats::format_text(&snapshot));
            println!("Dashboard: {}", path.display());
            if open {
                open_dashboard_in_browser(&path)?;
            }
        }

        "onboarding" => {
            let config = Config::load()?;
            let briefing = config.logs_dir.join("last-route.md");
            agent_brain::onboarding::print_onboarding(&config.home, briefing.is_file());
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
                            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
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
                            let report =
                                agent_brain::sync::cloud_pull(&engine, &brain_settings.sync.cloud)?;
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
                            let remote = flag_value(&args, "--remote").or_else(|| {
                                let r = brain_settings.sync.git.remote.clone();
                                if r.is_empty() {
                                    None
                                } else {
                                    Some(r)
                                }
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
                                    if r.is_empty() {
                                        None
                                    } else {
                                        Some(r)
                                    }
                                })
                                .context("usage: agent-brain sync git clone [--remote URL]")?;
                            let branch = brain_settings.sync.git.branch.clone();
                            let root =
                                agent_brain::sync::git_clone(&config.home, &remote, &branch)?;
                            println!("Cloned sync repo to {}", root.display());
                            eprintln!("Run: agent-brain sync git pull");
                        }
                        "push" => {
                            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
                            agent_brain::sync::git_push(
                                &store,
                                &config.home,
                                &brain_settings.sync.git,
                            )?;
                            println!(
                                "Pushed memory bundle to origin/{}",
                                brain_settings.sync.git.branch
                            );
                        }
                        "pull" => {
                            let engine = Arc::new(Engine::new(config)?);
                            let report =
                                agent_brain::sync::git_pull(&engine, &brain_settings.sync.git)?;
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
        "bootstrap" => {
            let remote = flag_value(&args, "--remote");
            let skip_install = args.iter().any(|a| a == "--skip-install");
            agent_brain::bootstrap_device::run(remote.as_deref(), skip_install)?;
        }
        "stop-gate" => {
            agent_brain::stop_gate::run_stop_gate()?;
        }
        "verify" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("");
            if sub == "init" {
                let force = args.iter().any(|a| a == "--force" || a == "-f");
                let target_dir = flag_value(&args, "--dir")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
                let path = agent_brain::verify::init_verify(&target_dir, force)?;
                println!("Initialized verification suite in {}", path.display());
                return Ok(());
            }
            let config = Config::load()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            if sub == "record" {
                let route_log_id = flag_value(&args, "--route-log-id");
                let cause = flag_value(&args, "--cause");
                let config_path = flag_value(&args, "--config").map(std::path::PathBuf::from);
                let scenarios = flag_value(&args, "--scenarios").map(std::path::PathBuf::from);
                let source_raw = flag_value(&args, "--source").unwrap_or_else(|| "hook".to_string());
                let source = agent_brain::verify::VerifySource::parse(&source_raw)?;
                if matches!(
                    source,
                    agent_brain::verify::VerifySource::Cli
                        | agent_brain::verify::VerifySource::Orchestrator
                ) {
                    anyhow::bail!(
                        "verify record cannot store cli/orchestrator evidence; run `agent-brain verify -- <cmd>` so the command executes"
                    );
                }
                let exit_code: i32 = flag_value(&args, "--exit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let cmd: Vec<String> = if let Some(idx) = args.iter().position(|a| a == "--") {
                    args[idx + 1..].to_vec()
                } else {
                    eprintln!(
                        "Usage: agent-brain verify record --exit N [--source hook|mcp|playbook|cli|orchestrator] [--config PATH] [--scenarios DIR] [--route-log-id ID] [--cause TAG] -- <cmd summary...>"
                    );
                    std::process::exit(1);
                };
                let report = agent_brain::verify::record_from_result(
                    &store,
                    exit_code,
                    &cmd.join(" "),
                    route_log_id.as_deref(),
                    cause.as_deref(),
                    config_path.as_deref(),
                    scenarios.as_deref(),
                    None,
                    source,
                )?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.success {
                    std::process::exit(report.exit_code);
                }
            } else {
                let route_log_id = flag_value(&args, "--route-log-id");
                let cause = flag_value(&args, "--cause");
                let config_path = flag_value(&args, "--config").map(std::path::PathBuf::from);
                let scenarios = flag_value(&args, "--scenarios").map(std::path::PathBuf::from);
                let source_raw =
                    flag_value(&args, "--source").unwrap_or_else(|| "cli".to_string());
                let source = agent_brain::verify::VerifySource::parse(&source_raw)?;
                let cmd: Vec<String> = if let Some(idx) = args.iter().position(|a| a == "--") {
                    args[idx + 1..].to_vec()
                } else {
                    eprintln!(
                        "Usage: agent-brain verify [--source cli|orchestrator] [--config PATH] [--scenarios DIR] [--route-log-id ID] [--cause TAG] -- <cmd> [args...]\n       agent-brain verify init [--force] [--dir DIR]\n       agent-brain verify record --exit N [--source ...] -- <cmd summary...>"
                    );
                    std::process::exit(1);
                };
                let report = agent_brain::verify::run_verify(
                    &store,
                    &cmd,
                    route_log_id.as_deref(),
                    cause.as_deref(),
                    config_path.as_deref(),
                    scenarios.as_deref(),
                    source,
                )?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.success {
                    std::process::exit(report.exit_code);
                }
            }
        }
        "report" => {
            let config = Config::load()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            agent_brain::verify::print_report(&store)?;
        }
        "doctor" => {
            let fix = args.iter().any(|a| a == "--fix");
            let mcp_only = args.iter().any(|a| a == "--mcp");
            if mcp_only {
                doctor::run_mcp_federation(fix)?;
            } else {
                doctor::run(fix)?;
            }
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
                    let used_by =
                        flag_value(&args, "--used-by").unwrap_or_else(|| "upstream_mcp".into());
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
        "sessions" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("help");
            let config = Config::load()?;
            config.ensure_dirs()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            match sub {
                "ingest" => {
                    let legacy = args.iter().any(|a| a == "--legacy");
                    let sources = parse_session_sources(&args)?;
                    let embedder = agent_brain::embed::Embedder::with_model(
                        agent_brain::embed::parse_embedding_model(&config.embedding_model),
                    )?;
                    let report = agent_brain::sessions::ingest_sessions_filtered(
                        &store, &embedder, &config, &sources, legacy,
                    )?;
                    if report.digests_stored > 0 || report.legacy_stored > 0 {
                        store.bump_index_version()?;
                    }
                    println!(
                        "session ingest: {} digests, {} legacy facts",
                        report.digests_stored, report.legacy_stored
                    );
                }
                "status" => {
                    let discoverable = agent_brain::sessions::discover_report(&config)?;
                    let stored = agent_brain::sessions::count_stored_digests_by_source(&store)?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "discoverable": discoverable,
                            "stored_digests_by_source": stored,
                            "opencode_db": agent_brain::sessions::opencode_db_path(
                                &agent_brain::sessions::session_scan_home(&config)
                                    .unwrap_or_else(|| config.home.clone()),
                            ).display().to_string(),
                        }))?
                    );
                }
                _ => {
                    eprintln!("Usage: agent-brain sessions ingest [--source cursor,codex,gemini,opencode] [--legacy]");
                    eprintln!("       agent-brain sessions status");
                    std::process::exit(1);
                }
            }
        }
        "promote" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let sub = args.get(2).map(String::as_str).unwrap_or("help");
            match sub {
                "list" => {
                    let status = flag_value(&args, "--status");
                    let rows = agent_brain::promote::list_staging(&store, status.as_deref())?;
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                }
                "approve" => {
                    let id = args
                        .get(3)
                        .context("usage: agent-brain promote approve <staging-id>")?;
                    let path = agent_brain::promote::approve_staging(&store, id)?;
                    let engine = Arc::new(Engine::new(config)?);
                    let n = engine.bootstrap(None)?;
                    println!("Approved skill at {}", path.display());
                    println!("Reindexed {n} items");
                }
                "reject" => {
                    let id = args
                        .get(3)
                        .context("usage: agent-brain promote reject <staging-id>")?;
                    agent_brain::promote::reject_staging(&store, id)?;
                    println!("Rejected staging {id}");
                }
                _ => {
                    eprintln!(
                        "Usage: agent-brain promote list [--status pending|approved|rejected]"
                    );
                    eprintln!("       agent-brain promote approve <staging-id>");
                    eprintln!("       agent-brain promote reject <staging-id>");
                    std::process::exit(1);
                }
            }
        }
        "memory" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let brain_settings = settings::AgentBrainSettings::load(&config.home);
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let sub = args.get(2).map(String::as_str).unwrap_or("help");
            match sub {
                "gc" => {
                    let dry_run = !args.iter().any(|a| a == "--apply");
                    let force = args.iter().any(|a| a == "--force");
                    let stale_days = flag_value(&args, "--stale-days")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(brain_settings.memory_gc.stale_days);
                    let very_stale_days = flag_value(&args, "--very-stale-days")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(brain_settings.memory_gc.very_stale_days);
                    let report = agent_brain::memory_gc::run_memory_gc_with_thresholds(
                        &store,
                        dry_run,
                        force,
                        stale_days,
                        very_stale_days,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    if dry_run && !report.ids.is_empty() {
                        eprintln!("Dry run — re-run with --apply to archive.");
                    }
                }
                "observe" => {
                    let dry_run = args.iter().any(|a| a == "--dry-run");
                    let embedder = agent_brain::embed::Embedder::with_model(
                        agent_brain::embed::parse_embedding_model(&config.embedding_model),
                    )?;
                    let cfg = agent_brain::observation::ObservationConfig {
                        min_facts_per_topic: brain_settings.observation.min_facts_per_topic,
                        window_days: brain_settings.observation.window_days,
                    };
                    let report = agent_brain::observation::run_observations(
                        &store, &embedder, &cfg, dry_run,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                "extract" => {
                    let dry_run = args.iter().any(|a| a == "--dry-run");
                    let explain = args.iter().any(|a| a == "--explain");
                    if explain {
                        let explanations = agent_brain::trace_extract::explain_pending_traces(
                            &store,
                            &config.home,
                        )?;
                        println!("{}", serde_json::to_string_pretty(&explanations)?);
                        return Ok(());
                    }
                    let embedder = agent_brain::embed::Embedder::with_model(
                        agent_brain::embed::parse_embedding_model(&config.embedding_model),
                    )?;
                    let cfg = agent_brain::trace_extract::TraceExtractConfig {
                        confidence: brain_settings.trace_extract.confidence,
                        explain: false,
                    };
                    let report = agent_brain::trace_extract::run_trace_extract(
                        &store,
                        &embedder,
                        &config.home,
                        &cfg,
                        dry_run,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                "put" => {
                    let topic = flag_value(&args, "--topic").context("memory put requires --topic")?;
                    let fact = flag_value(&args, "--fact").context("memory put requires --fact")?;
                    let scope = flag_value(&args, "--scope").unwrap_or_else(|| "project".into());
                    let scope_key = flag_value(&args, "--scope-key");
                    let confidence = flag_value(&args, "--confidence")
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.9);
                    let embedder = agent_brain::embed::Embedder::with_model(
                        agent_brain::embed::parse_embedding_model(&config.embedding_model),
                    )?;
                    let report = agent_brain::memory_pending::put_now(
                        &store,
                        &embedder,
                        &topic,
                        &fact,
                        &scope,
                        scope_key.as_deref(),
                        confidence,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                "enqueue" => {
                    let topic =
                        flag_value(&args, "--topic").context("memory enqueue requires --topic")?;
                    let fact =
                        flag_value(&args, "--fact").context("memory enqueue requires --fact")?;
                    let scope = flag_value(&args, "--scope").unwrap_or_else(|| "project".into());
                    let scope_key = flag_value(&args, "--scope-key");
                    let confidence = flag_value(&args, "--confidence")
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.9);
                    let path = agent_brain::memory_pending::enqueue(
                        &config.home,
                        &topic,
                        &fact,
                        &scope,
                        scope_key.as_deref(),
                        confidence,
                    )?;
                    println!("enqueued -> {}", path.display());
                    eprintln!("Flush with: agent-brain memory flush-pending");
                }
                "flush-pending" => {
                    let retries = flag_value(&args, "--retries")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(agent_brain::memory_pending::DEFAULT_RETRIES);
                    let backoff_ms = flag_value(&args, "--backoff-ms")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(agent_brain::memory_pending::DEFAULT_BACKOFF_MS);
                    let watch = args.iter().any(|a| a == "--watch");
                    let interval = flag_value(&args, "--interval-secs")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(5);
                    let max_rounds = flag_value(&args, "--max-rounds")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(if watch { 24 } else { 1 });
                    let embedder = agent_brain::embed::Embedder::with_model(
                        agent_brain::embed::parse_embedding_model(&config.embedding_model),
                    )?;
                    let report = if watch {
                        agent_brain::memory_pending::flush_watch(
                            &config.home,
                            &store,
                            &embedder,
                            retries,
                            backoff_ms,
                            interval,
                            max_rounds,
                        )?
                    } else {
                        agent_brain::memory_pending::flush_pending(
                            &config.home,
                            &store,
                            &embedder,
                            retries,
                            backoff_ms,
                        )?
                    };
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    if report.remaining > 0 {
                        std::process::exit(2);
                    }
                }
                _ => {
                    eprintln!(
                        "Usage: agent-brain memory gc [--apply] [--force] [--stale-days N] [--very-stale-days N]"
                    );
                    eprintln!("       agent-brain memory observe [--dry-run]");
                    eprintln!("       agent-brain memory extract [--dry-run] [--explain]");
                    eprintln!("       agent-brain memory put --topic T --fact F [--scope project|global] [--scope-key K]");
                    eprintln!("       agent-brain memory enqueue --topic T --fact F   # MCP fallback queue");
                    eprintln!("       agent-brain memory flush-pending [--retries N] [--watch] [--interval-secs N]");
                    std::process::exit(1);
                }
            }
        }
        "gc" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let min_confidence = flag_value(&args, "--min-confidence")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.3);
            let max_age_days = flag_value(&args, "--max-age-days")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(90);
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let stats = agent_brain::gc::run_gc(&store, min_confidence, max_age_days)?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        "log" => {
            let list = args.iter().any(|a| a == "--list" || a == "-l");
            let follow = args.iter().any(|a| a == "--follow" || a == "-f");
            if list {
                let logs = agent_brain::log::list_logs()?;
                if logs.is_empty() {
                    println!("No log files found.");
                } else {
                    println!("Available logs:");
                    for log in &logs {
                        println!("  {log}");
                    }
                }
            } else {
                let name = args
                    .get(2)
                    .context("usage: agent-brain log <name> [--follow] [--list]")?;
                if follow {
                    agent_brain::log::follow_log(name)?;
                } else {
                    agent_brain::log::print_log(name)?;
                }
            }
        }
        "help" | "--help" | "-h" => cli_help::print_root(),
        _ => {
            eprintln!("Unknown command: {cmd}");
            cli_help::print_root();
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

fn open_dashboard_in_browser(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).status()?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).status();
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status()?;
    }
    Ok(())
}

fn parse_session_sources(args: &[String]) -> Result<Vec<agent_brain::sessions::SessionSource>> {
    let Some(raw) = flag_value(args, "--source") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for part in raw.split(',') {
        let src = agent_brain::sessions::SessionSource::parse(part.trim())
            .with_context(|| format!("unknown session source: {}", part.trim()))?;
        if !out.contains(&src) {
            out.push(src);
        }
    }
    Ok(out)
}

fn handle_package_add(args: &[String], source_index: usize) -> Result<()> {
    let source = args
        .get(source_index)
        .context("missing package source (@alias, owner/repo, or GitHub URL)")?;
    let git_ref = flag_value(args, "--ref");
    let skip_index = args.iter().any(|a| a == "--no-index");
    let config = Config::load()?;
    if let Some(resolved) = packages::lookup_alias(source)? {
        match resolved {
            packages::ResolvedAlias::Bundle(name) => {
                let record = packages::install_bundled(&config, &name)?;
                println!(
                    "Installed bundled package '{}' ({})",
                    record.name, record.source
                );
            }
            packages::ResolvedAlias::Git(sources) => {
                for src in &sources {
                    let record = packages::add_package(&config, src, git_ref.as_deref())?;
                    println!(
                        "Installed package '{}' from {} ({})",
                        record.name,
                        record.source,
                        record.commit.unwrap_or_else(|| "unknown".into())
                    );
                }
            }
        }
    } else {
        let record = packages::add_package(&config, source, git_ref.as_deref())?;
        println!(
            "Installed package '{}' from {} ({})",
            record.name,
            record.source,
            record.commit.unwrap_or_else(|| "unknown".into())
        );
    }
    if !skip_index {
        let mut config = config;
        config.bootstrap_background = false;
        config.session_ingest_background = false;
        let engine = Arc::new(Engine::new(config)?);
        let n = engine.bootstrap(None)?;
        println!("Indexed {n} items");
    }
    Ok(())
}
