use anyhow::{Context, Result};
use tekmor::{config::Config, host_hooks};
use tracing_subscriber::EnvFilter;

mod cli_help;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tekmor=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
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

    let cmd = args.get(1).map(String::as_str).unwrap_or("help");

    match cmd {
        "install" => {
            let user = args.iter().any(|a| a == "--global" || a == "-g");
            let quiet = args.iter().any(|a| a == "--quiet");
            host_hooks::install_claude_code_hooks(user, quiet)?;
        }
        "version" => {
            println!("tekmor {}", env!("CARGO_PKG_VERSION"));
        }
        "stop-gate" => {
            tekmor::stop_gate::run_stop_gate()?;
        }
        "commit-gate" => {
            tekmor::commit_gate::run_commit_gate()?;
        }
        "bash-gate" => {
            tekmor::command_policy::run_bash_gate()?;
        }
        "rules" => {
            tekmor::command_rules::run_cli(&args[2..])?;
        }
        "verify" => {
            let sub = args.get(2).map(String::as_str).unwrap_or("");
            if sub == "init" {
                let force = args.iter().any(|a| a == "--force" || a == "-f");
                let target_dir = flag_value(&args, "--dir")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                    });
                let path = tekmor::verify::init_verify(&target_dir, force)?;
                println!("Initialized verification suite in {}", path.display());
                if tekmor::verify::load_command_allowlist(&target_dir).is_empty() {
                    println!("no test command detected; add yours to .verify/verify.toml");
                }
                return Ok(());
            }
            let config = Config::load()?;
            let store = tekmor::db::store::BrainStore::open(&config.db_path)?;
            if sub == "run" {
                let source_raw = flag_value(&args, "--source").unwrap_or_else(|| "cli".to_string());
                let source = tekmor::verify::VerifySource::parse(&source_raw)?;
                let root = match flag_value(&args, "--project") {
                    Some(path) => std::path::PathBuf::from(path),
                    None => {
                        let cwd = std::env::current_dir()?;
                        tekmor::config::find_repo_root(&cwd).unwrap_or_else(|| cwd.clone())
                    }
                };
                match tekmor::verify::run_default_step(&store, &root, source)? {
                    None => eprintln!("no [verify.default] step declared in {}", root.display()),
                    Some(run) => {
                        let report = &run.report;
                        if args.iter().any(|a| a == "--json") {
                            println!("{}", serde_json::to_string_pretty(report)?);
                        } else {
                            let verdict = if report.success { "passed" } else { "failed" };
                            println!(
                                "verify default {verdict} (exit {}): {}",
                                report.exit_code, report.cmd
                            );
                        }
                        if !report.success {
                            eprintln!("{}", run.output.trim_end());
                            std::process::exit(report.exit_code);
                        }
                    }
                }
                return Ok(());
            }
            if sub == "summary" {
                let summaries = tekmor_core::evidence::summarize(
                    &tekmor::evidence_reader::recent_runs(&store, 500)?,
                );
                if args.iter().any(|a| a == "--json") {
                    println!("{}", serde_json::to_string_pretty(&summaries)?);
                } else {
                    for s in &summaries {
                        println!(
                            "{}/{} failed  last exit {}  {}",
                            s.failures, s.runs, s.last_exit_code, s.cmd
                        );
                    }
                }
                return Ok(());
            }
            if sub == "receipt" {
                let root = match flag_value(&args, "--project") {
                    Some(path) => std::path::PathBuf::from(path),
                    None => {
                        let cwd = std::env::current_dir()?;
                        tekmor::config::find_repo_root(&cwd).unwrap_or(cwd)
                    }
                };
                let receipt = tekmor::receipt::receipt_for_project(&store, &root)?;
                if args.iter().any(|a| a == "--json") {
                    println!("{}", serde_json::to_string_pretty(&receipt)?);
                } else if args.iter().any(|a| a == "--markdown") {
                    print!("{}", tekmor::receipt::render_markdown(&receipt));
                } else {
                    print!("{}", tekmor::receipt::render_text(&receipt));
                }
                if receipt.verdict != tekmor::receipt::Verdict::Passed {
                    std::process::exit(1);
                }
                return Ok(());
            }
            if sub == "record" {
                let route_log_id = flag_value(&args, "--route-log-id");
                let cause = flag_value(&args, "--cause");
                let config_path = flag_value(&args, "--config").map(std::path::PathBuf::from);
                let scenarios = flag_value(&args, "--scenarios").map(std::path::PathBuf::from);
                let source_raw =
                    flag_value(&args, "--source").unwrap_or_else(|| "hook".to_string());
                let source = tekmor::verify::VerifySource::parse(&source_raw)?;
                if matches!(
                    source,
                    tekmor::verify::VerifySource::Cli
                        | tekmor::verify::VerifySource::Orchestrator
                ) {
                    anyhow::bail!(
                        "verify record cannot store cli/orchestrator evidence; run `tekmor verify -- <cmd>` so the command executes"
                    );
                }
                let exit_code: i32 = flag_value(&args, "--exit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let cmd: Vec<String> = if let Some(idx) = args.iter().position(|a| a == "--") {
                    args[idx + 1..].to_vec()
                } else {
                    eprintln!(
                        "Usage: tekmor verify record --exit N [--source hook|mcp|playbook|cli|orchestrator] [--config PATH] [--scenarios DIR] [--route-log-id ID] [--cause TAG] -- <cmd summary...>"
                    );
                    std::process::exit(1);
                };
                let report = tekmor::verify::record_from_result(
                    &store,
                    exit_code,
                    &cmd.join(" "),
                    tekmor::verify::VerifyRecord {
                        route_log_id: route_log_id.as_deref(),
                        cause_tag: cause.as_deref(),
                        config_path: config_path.as_deref(),
                        scenarios_dir: scenarios.as_deref(),
                        cwd: None,
                        source,
                    },
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
                let source_raw = flag_value(&args, "--source").unwrap_or_else(|| "cli".to_string());
                let source = tekmor::verify::VerifySource::parse(&source_raw)?;
                let save_artifact = args.iter().any(|a| a == "--save-artifact");
                let cmd: Vec<String> = if let Some(idx) = args.iter().position(|a| a == "--") {
                    args[idx + 1..].to_vec()
                } else {
                    eprintln!(
                        "Usage: tekmor verify [--source cli|orchestrator] [--config PATH] [--scenarios DIR] [--route-log-id ID] [--cause TAG] [--save-artifact] -- <cmd> [args...]\n       tekmor verify init [--force] [--dir DIR]\n       tekmor verify record --exit N [--source ...] -- <cmd summary...>"
                    );
                    std::process::exit(1);
                };
                let report = tekmor::verify::run_verify(
                    &store,
                    &cmd,
                    tekmor::verify::VerifyRun {
                        route_log_id: route_log_id.as_deref(),
                        cause_tag: cause.as_deref(),
                        config_path: config_path.as_deref(),
                        scenarios_dir: scenarios.as_deref(),
                        source,
                        save_artifact,
                    },
                )?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.success {
                    std::process::exit(report.exit_code);
                }
            }
        }
        "help" | "--help" | "-h" => cli_help::print_root(),
        "ext" => {
            let config = Config::load()?;
            match args.get(2).map(String::as_str).unwrap_or("list") {
                "list" => {
                    for name in tekmor::extensions::installed(&config.home)? {
                        println!("{name}");
                    }
                }
                "add" => {
                    let name = args.get(3).context("usage: tekmor ext add <name>")?;
                    let path = tekmor::extensions::install(&config.home, name)?;
                    println!("installed {name} -> {}", path.display());
                }
                "remove" => {
                    let name = args
                        .get(3)
                        .context("usage: tekmor ext remove <name>")?;
                    tekmor::extensions::remove(&config.home, name)?;
                    println!("removed {name}");
                }
                _ => {
                    eprintln!("Usage: tekmor ext [list | add <name> | remove <name>]");
                    std::process::exit(1);
                }
            }
        }
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

