use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use agent_brain::{
    auto_update, config::Config, doctor, engine::Engine, grpc, host_install, install, mcp,
    packages, serve_meta, settings,
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
            if let Err(e) = agent_brain::autowire::auto_wire(
                &std::env::current_exe()?.display().to_string(),
                env!("CARGO_PKG_VERSION"),
            ) {
                tracing::warn!("Auto-wire failed: {}", e);
            }
            mcp::run_stdio(engine).await?;
        }
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
                tracing::info!("indexed {n} items");
            }
            grpc::serve(engine, addr).await?;
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
            let source = args
                .get(2)
                .context("missing package source (@alias, owner/repo, or GitHub URL)")?;
            let git_ref = flag_value(&args, "--ref");
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
        }
        "registry" => match args.get(2).map(String::as_str).unwrap_or("list") {
            "list" => {
                for entry in packages::list_aliases()? {
                    let source = if let Some(bundle) = &entry.bundle {
                        format!("bundle:{bundle}")
                    } else {
                        entry.packages.join(", ")
                    };
                    println!("@{:<10}  {}  [{}]", entry.alias, entry.description, source);
                }
            }
            other => {
                eprintln!("Usage: agent-brain registry list");
                if !other.is_empty() {
                    std::process::exit(1);
                }
            }
        },
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
            let target = host_install::HostTarget::from_args(&args);
            let reload = args.iter().any(|a| a == "--reload");
            let print_only = args.iter().any(|a| a == "--print-only");
            let global = args.iter().any(|a| a == "--global");
            install::run(target, print_only, reload)?;
            if (global || matches!(target, host_install::HostTarget::All)) && !print_only {
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
                    println!("Dismissed pending anti-pattern suggestion");
                }
                "show" | _ => match agent_brain::route_briefing::read_anti_pattern_suggestion(
                    &Config::load()?.home,
                ) {
                    Some(suggestion) => {
                        println!("{}", serde_json::to_string_pretty(&suggestion)?);
                        println!("\nApprove: agent-brain suggest-memory approve");
                    }
                    None => {
                        eprintln!("No anti-pattern suggestion pending.");
                        std::process::exit(1);
                    }
                },
            }
        }
        "onboarding" => {
            let config = Config::load()?;
            let briefing = config.home.join("logs").join("last-route.md");
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
                _ => {
                    eprintln!(
                        "Usage: agent-brain memory gc [--apply] [--force] [--stale-days N] [--very-stale-days N]"
                    );
                    eprintln!("       agent-brain memory observe [--dry-run]");
                    eprintln!("       agent-brain memory extract [--dry-run] [--explain]");
                    std::process::exit(1);
                }
            }
        }
        "dataset" => {
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
                    let entries = agent_brain::dataset::export_dataset(
                        &store,
                        min_confidence,
                        only_successful,
                    )?;
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
                    let entries = agent_brain::dataset::export_dataset(
                        &store,
                        min_confidence,
                        true,
                    )?;
                    let stats = agent_brain::dataset::compute_stats(&entries);
                    println!("{}", serde_json::to_string_pretty(&stats)?);
                }
                _ => {
                    eprintln!("Usage: agent-brain dataset export [--out PATH] [--min-confidence N] [--all-outcomes]");
                    eprintln!("       agent-brain dataset stats [--min-confidence N]");
                    std::process::exit(1);
                }
            }
        }
        "digest" => {
            if !args.iter().any(|a| a == "--weekly") {
                eprintln!("Usage: agent-brain digest --weekly");
                std::process::exit(1);
            }
            let config = Config::load()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let digest = agent_brain::operator_digest::weekly_digest(&store, 7)?;
            println!(
                "{}",
                agent_brain::operator_digest::format_weekly_digest(&digest)
            );
        }
        "stats" => {
            let json = args.iter().any(|a| a == "--json");
            let days = flag_value(&args, "--days")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(7);
            let config = Config::load()?;
            config.ensure_dirs()?;
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
            config.ensure_dirs()?;
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let snapshot = agent_brain::stats::collect(&store, &config, days)?;
            let path = agent_brain::dashboard::write_dashboard_html(&config.home, &snapshot)?;
            println!("Dashboard written: {}", path.display());
            if open {
                open_dashboard_in_browser(&path)?;
            }
        }
        "eval" => {
            let live = args.iter().any(|a| a == "--live");
            let skills_sh = args.iter().any(|a| a == "--skills-sh");
            if skills_sh {
                let seed_runtime = args.iter().any(|a| a == "--seed");
                let snapshot = flag_value(&args, "--snapshot")
                    .map(PathBuf::from)
                    .unwrap_or_else(agent_brain::skills_sh::default_snapshot_path);
                let golden = flag_value(&args, "--golden")
                    .map(PathBuf::from)
                    .unwrap_or_else(agent_brain::skills_sh::default_golden_path);
                let fixture_db: Option<PathBuf> = if seed_runtime {
                    None
                } else {
                    flag_value(&args, "--fixture-db")
                        .map(PathBuf::from)
                        .or_else(|| {
                            let default = agent_brain::fixture::default_fixture_2k_path();
                            default.exists().then_some(default)
                        })
                };
                let report = agent_brain::skills_sh::run_skills_sh_eval(
                    &snapshot,
                    &golden,
                    fixture_db.as_deref(),
                )?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if let Err(err) = agent_brain::skills_sh::assert_skills_sh_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            } else if args.iter().any(|a| a == "--beam") {
                let report = agent_brain::beam_eval::run_beam_eval_isolated()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Err(err) = agent_brain::beam_eval::assert_beam_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            } else if !args.iter().any(|a| a == "--ci") {
                eprintln!("Usage: agent-brain eval --ci [--live]");
                eprintln!(
                    "       agent-brain eval --beam              BEAM memory+routing harness"
                );
                eprintln!("       agent-brain eval --skills-sh [--fixture-db PATH] [--seed] [--write PATH]");
                eprintln!("  --ci            Run Recall@3 gate (default: isolated fixture DB)");
                eprintln!("  --live          Use ~/.agent_brain brain.db (not for CI)");
                eprintln!("  --skills-sh     Run skills.sh Recall@3 (~2000 index)");
                eprintln!("  --fixture-db    Committed benchmark DB (default: docs/benchmarks/fixture-2k.db)");
                eprintln!(
                    "  --seed          Seed index at runtime from snapshot instead of fixture DB"
                );
                std::process::exit(1);
            } else {
                let report = if live {
                    let mut config = Config::load()?;
                    config.bootstrap_background = false;
                    config.session_ingest_background = false;
                    let engine = Arc::new(Engine::new(config)?);
                    agent_brain::eval::run_ci_eval(&engine)?
                } else {
                    agent_brain::eval::run_ci_eval_isolated()?
                };
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Err(err) = agent_brain::eval::assert_ci_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            }
        }
        "fixture" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("");
            match sub {
                "build" => {
                    let snapshot = flag_value(&args, "--snapshot")
                        .map(PathBuf::from)
                        .unwrap_or_else(agent_brain::skills_sh::default_snapshot_path);
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(agent_brain::fixture::default_fixture_2k_path);
                    let index_size = flag_value(&args, "--index-size")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(agent_brain::skills_sh::SKILLS_SH_SIMULATED_INDEX);
                    let allow_fillers = args.iter().any(|a| a == "--allow-fillers");
                    let report = agent_brain::skills_sh::build_fixture_db(
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
                        .unwrap_or_else(agent_brain::fixture::default_fixture_2k_path);
                    let meta = agent_brain::fixture::read_fixture_meta(&path)?;
                    let breakdown = {
                        let dir = tempfile::tempdir()?;
                        let config = Config::isolated(dir.path().to_path_buf());
                        config.ensure_dirs()?;
                        std::fs::copy(&path, &config.db_path)?;
                        let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
                        agent_brain::fixture::fixture_db_breakdown(&store)?
                    };
                    #[derive(serde::Serialize)]
                    struct VerifyOut {
                        meta: agent_brain::fixture::FixtureDbMeta,
                        breakdown: agent_brain::fixture::FixtureDbBreakdown,
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
            let sub = args.get(2).map(String::as_str).unwrap_or("");
            match sub {
                "sync" => {
                    let manifest_path = flag_value(&args, "--manifest")
                        .map(PathBuf::from)
                        .unwrap_or_else(agent_brain::skills_sh::default_manifest_path);
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(agent_brain::skills_sh::default_snapshot_path);
                    let target = flag_value(&args, "--target")
                        .or_else(|| flag_value(&args, "--max"))
                        .and_then(|v| v.parse().ok());
                    let required_only = args.iter().any(|a| a == "--required-only");
                    let merge = args.iter().any(|a| a == "--merge");
                    let mut manifest = agent_brain::skills_sh::load_manifest(&manifest_path)?;
                    if required_only {
                        manifest.discovery_queries.clear();
                        manifest.max_skills = manifest.required_ids.len();
                    }
                    let delay_ms = flag_value(&args, "--delay-ms")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(if required_only { 20_000 } else { 400 });
                    let mut options = agent_brain::skills_sh::SyncOptions::from_manifest(
                        &manifest, target, delay_ms,
                    );
                    if merge || write_path.exists() {
                        options.merge_path = Some(write_path.clone());
                    }
                    options.checkpoint_path = Some(write_path.clone());
                    let (snapshot, report) =
                        agent_brain::skills_sh::sync_snapshot_with_options(&manifest, options)?;
                    agent_brain::skills_sh::write_snapshot(&write_path, &snapshot)?;
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
                        .unwrap_or_else(agent_brain::skills_sh::default_snapshot_path);
                    let delay_ms = flag_value(&args, "--delay-ms")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(200);
                    let max_retries = flag_value(&args, "--max").and_then(|v| v.parse().ok());
                    let download_attempts = flag_value(&args, "--download-attempts")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(2);
                    let (snapshot, report) = agent_brain::skills_sh::retry_failed_downloads(
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
                        .unwrap_or_else(agent_brain::fixture::default_fixture_2k_path);
                    let snapshot = flag_value(&args, "--snapshot")
                        .map(PathBuf::from)
                        .unwrap_or_else(agent_brain::skills_sh::default_snapshot_path);
                    let write_path = flag_value(&args, "--write")
                        .map(PathBuf::from)
                        .unwrap_or_else(agent_brain::skills_sh::default_golden_path);
                    let target = flag_value(&args, "--target")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(25);
                    let golden =
                        agent_brain::skills_sh::probe_golden_cases(&fixture, &snapshot, target)?;
                    agent_brain::skills_sh::write_golden(&write_path, &golden)?;
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
        "bench" => {
            let onnx = args.iter().any(|a| a == "--onnx");
            let ci = args.iter().any(|a| a == "--ci");
            let supervisor = args.iter().any(|a| a == "--supervisor");
            let scale = args.iter().any(|a| a == "--scale");
            let graphify = args.iter().any(|a| a == "--graphify");
            let mcp = args.iter().any(|a| a == "--mcp");
            if mcp {
                let report = agent_brain::mcp_bench::run_mcp_bench()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) = agent_brain::mcp_bench::assert_mcp_bench_gate(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if graphify {
                let full = args.iter().any(|a| a == "--full");
                let sizes: &[usize] = if full {
                    agent_brain::graphify_bench::GRAPHIFY_SIZES
                } else {
                    &[1_000]
                };
                let report = agent_brain::graphify_bench::run_graphify_bench(sizes)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) =
                        agent_brain::graphify_bench::assert_graphify_bench_gate(&report)
                    {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if supervisor {
                let report = agent_brain::supervisor_bench::run_supervisor_bench()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) =
                        agent_brain::supervisor_bench::assert_supervisor_bench_gate(&report)
                    {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if onnx {
                let fixture = flag_value(&args, "--fixture-db")
                    .map(PathBuf::from)
                    .unwrap_or_else(agent_brain::fixture::default_fixture_2k_path);
                let report = agent_brain::bench::run_onnx_fixture_bench(&fixture)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert-target") {
                    if let Err(err) = agent_brain::bench::assert_onnx_bench_target(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if scale {
                let full = args.iter().any(|a| a == "--full");
                let sizes: &[usize] = if full {
                    agent_brain::scale_bench::SCALE_SIZES
                } else {
                    &[1_000]
                };
                let report = agent_brain::scale_bench::run_scale_bench(sizes)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Some(path) = flag_value(&args, "--write") {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(&path, format!("{json}\n"))?;
                }
                if args.iter().any(|a| a == "--assert") {
                    if let Err(err) = agent_brain::scale_bench::assert_scale_bench_gate(&report) {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                }
            } else if ci {
                let report = agent_brain::bench::run_ci_bench()?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if let Err(err) = agent_brain::bench::assert_bench_gate(&report) {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            } else {
                eprintln!("Usage: agent-brain bench --ci");
                eprintln!("       agent-brain bench --mcp [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --graphify [--full] [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --scale [--full] [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --supervisor [--assert] [--write PATH]");
                eprintln!("       agent-brain bench --onnx [--fixture-db PATH] [--write PATH] [--assert-target]");
                std::process::exit(1);
            }
        }
        "proofs" => {
            if !args.iter().any(|a| a == "--ci") {
                eprintln!("Usage: agent-brain proofs --ci [--write PATH]");
                std::process::exit(1);
            }
            let report = agent_brain::proofs::run_ci_proofs()?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if let Some(path) = flag_value(&args, "--write") {
                agent_brain::proofs::write_proof_report(PathBuf::from(path).as_path(), &report)?;
            }
            if let Ok(config) = Config::load() {
                let _ = agent_brain::stats::persist_proof_snapshot(&config.home, &report);
            }
            if let Err(err) = agent_brain::proofs::assert_ci_proofs(&report) {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        "distill" => {
            let config = Config::load()?;
            config.ensure_dirs()?;
            let out = flag_value(&args, "--out")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("ARCHITECTURE.md"));
            let store = agent_brain::db::store::BrainStore::open(&config.db_path)?;
            let distilled = agent_brain::distill::distill(&store)?;
            agent_brain::distill::write_architecture_md(&distilled, &out)?;
            println!("Architecture written to {}", out.display());
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

fn print_usage() {
    eprintln!(
        r#"agent-brain — local MCP router for agents, skills, rules, and memory

Usage:
  agent-brain serve                         Start MCP server (stdio)
  agent-brain grpc serve [--addr HOST:PORT] Orchestrator bridge API (gRPC, default 127.0.0.1:7842)
  agent-brain index                         Reindex local agents/skills/rules/memory
  agent-brain add <@alias|owner/repo|url>   Install package(s) and index
  agent-brain add @supervisor               Execution supervisor pack (token-efficient ops)
  agent-brain add @starter                  Curated onboarding pack
  agent-brain add @nextjs                   Vercel React/Next.js skills
  agent-brain registry list                 Show curated @aliases
  agent-brain package list                  List installed packages
  agent-brain package update [name]         Update one or all packages
  agent-brain package remove <name>         Remove an installed package
  agent-brain install [--global] [--reload]     Cursor MCP (default)
  agent-brain install --claude-desktop          Claude Desktop MCP config
  agent-brain install --vscode [--global]       VS Code workspace or user mcp.json
  agent-brain install --claude-code [--global]  Claude Code .mcp.json or ~/.claude.json
  agent-brain install --opencode [--global]     OpenCode opencode.json (project or ~/.config/opencode)
  agent-brain install --codex [--global]        Codex config.toml + hooks.json (project or ~/.codex)
  agent-brain install --gemini [--global]       Gemini CLI settings.json (project or ~/.gemini)
  agent-brain install --antigravity [--global]  Antigravity mcp_config.json (project or ~/.gemini)
  agent-brain install --all                     All of the above (user/global scope)
  agent-brain update [--force] [--mcp-only]     Run auto-update (MCP uses release redirect; API fallback respects GITHUB_TOKEN/GH_TOKEN)
  agent-brain config init                     Write ~/.agent_brain/config.yaml defaults
  agent-brain config show                     Print active config file
  agent-brain version                         Print installed version
  agent-brain briefing                        Print last human-readable route summary
  agent-brain suggest-memory [approve|reject] Show or promote hook anti-pattern to store_memory
  agent-brain learn url <URL> [--topic NAME] [--dry-run]  Ingest allowlisted docs into skills + memory
  agent-brain learn allowlist                          Show docs.allowed_domains from config
  agent-brain graphify enable|disable|status|ingest|run|query  Graphify orchestration (codebase graph)
  agent-brain stats [--days N] [--json]       Index, routing, token savings, adoption milestones
  agent-brain dashboard [--days N] [--open]   Local HTML value dashboard (token ROI, memories)
  agent-brain dataset export [--out PATH]     Export trajectories as JSONL training dataset
  agent-brain dataset stats                   Compute stats on exported trajectories
  agent-brain digest --weekly                 Operator digest from retrieval_log
  agent-brain onboarding                      USP + 5-minute getting started checklist
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
  agent-brain sessions ingest [--source SRCS] [--legacy]  Import session digests (cursor/codex/gemini/opencode)
  agent-brain sessions status                 Discoverable vs stored session digests
  agent-brain promote list|approve|reject     Skill promotion workflow (human approval required)
  agent-brain memory gc [--apply] [--force] [--stale-days N] [--very-stale-days N]  Archive stale facts (dry-run by default)
  agent-brain digest --weekly                 Operator digest from retrieval_log
  agent-brain eval --ci [--live]                 Recall@3 gate (isolated fixture; --live uses brain.db)
  agent-brain bench --ci                         Latency gate on 500-skill fixture (isolated)
  agent-brain bench --mcp [--assert]             Full MCP tool latency report (route, context, token tools, graphify)
  agent-brain bench --graphify [--full] [--assert]  Graphify ingest + code_context route bench
  agent-brain bench --scale [--full] [--assert]  ANN scale bench at 1k/5k/10k (p95 ≤ 50ms)
  agent-brain bench --supervisor [--assert]      Supervisor skill/must_apply/savings bench
  agent-brain bench --onnx [--fixture-db PATH]   ONNX warm-route bench on fixture-2k.db (nightly)
  agent-brain eval --skills-sh [--fixture-db PATH]   skills.sh Recall@3 (~2000 index)
  agent-brain fixture build [--index-size N]      Build committed fixture-2k.db from snapshot
  agent-brain skills-sh sync [--max N]          Refresh skills.sh snapshot from public API
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
