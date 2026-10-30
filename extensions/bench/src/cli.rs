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
                    .unwrap_or_else(agent_brain::fixture::default_fixture_2k_path);
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
                eprintln!("       agent-brain bench --beam              BEAM memory+routing harness");
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
