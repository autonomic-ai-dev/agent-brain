//! CLI command handlers for `agent-brain mcp` (list, add, remove, sync, test).

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::engine::Engine;
use crate::mcp_federation::persist_brain_settings;
use crate::settings::{AgentBrainSettings, UpstreamServerConfig};
use crate::upstream::{list_server_tools, refresh_upstream_index};

pub async fn handle_mcp_command(args: &[String]) -> Result<()> {
    let sub = args.get(2).map(String::as_str).unwrap_or("list");
    let config = Config::load()?;
    let mut settings = AgentBrainSettings::load(&config.home);

    match sub {
        "list" | "ls" => {
            println!(
                "{:<18} {:<10} {:<8} {}",
                "SERVER", "STATUS", "TOOLS", "COMMAND"
            );
            println!("{:-<18} {:-<10} {:-<8} {:-<30}", "", "", "", "");

            let engine = Engine::new(config)?;
            let indexed_tools = engine.store.list_upstream_tools().unwrap_or_default();

            if settings.upstream_mcp.servers.is_empty() {
                println!("No upstream MCP servers configured. Register one with: agent-brain mcp add <name> -- <command>");
                return Ok(());
            }

            for server in &settings.upstream_mcp.servers {
                let status = if server.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let tool_count = indexed_tools
                    .iter()
                    .filter(|t| t.server.eq_ignore_ascii_case(&server.name))
                    .count();
                let cmd_display = if server.args.is_empty() {
                    server.command.clone()
                } else {
                    format!("{} {}", server.command, server.args.join(" "))
                };
                println!(
                    "{:<18} {:<10} {:<8} {}",
                    server.name, status, tool_count, cmd_display
                );
            }
        }
        "add" => {
            let name = args.get(3).context(
                "usage: agent-brain mcp add <name> [--env KEY=VAL] -- <command> [args...]",
            )?;
            if name.starts_with('-') {
                bail!("server name cannot start with a dash; usage: agent-brain mcp add <name> -- <command>");
            }

            // Find "--" separator
            let dash_pos = args.iter().position(|a| a == "--");
            let (flags_part, cmd_part) = match dash_pos {
                Some(pos) => {
                    let flags = if pos > 4 { &args[4..pos] } else { &args[0..0] };
                    let cmd = &args[pos + 1..];
                    (flags, cmd)
                }
                None => {
                    if args.len() <= 4 {
                        bail!("missing command; usage: agent-brain mcp add <name> [--env KEY=VAL] -- <command> [args...]");
                    }
                    (&args[0..0], &args[4..])
                }
            };

            if cmd_part.is_empty() {
                bail!("command cannot be empty; usage: agent-brain mcp add <name> -- <command> [args...]");
            }

            let mut env = HashMap::new();
            for chunk in flags_part {
                if let Some(rest) = chunk.strip_prefix("--env=") {
                    if let Some((k, v)) = rest.split_once('=') {
                        env.insert(k.to_string(), v.to_string());
                    }
                }
            }

            let command = cmd_part[0].clone();
            let server_args = cmd_part[1..].to_vec();

            let verb = if let Some(existing) = settings
                .upstream_mcp
                .servers
                .iter_mut()
                .find(|s| s.name.eq_ignore_ascii_case(name))
            {
                existing.command = command.clone();
                existing.args = server_args.clone();
                if !env.is_empty() {
                    existing.env = env.clone();
                }
                existing.enabled = true;
                "Updated"
            } else {
                settings.upstream_mcp.servers.push(UpstreamServerConfig {
                    name: name.clone(),
                    command: command.clone(),
                    args: server_args.clone(),
                    env,
                    enabled: true,
                });
                "Registered"
            };

            settings.upstream_mcp.enabled = true;
            persist_brain_settings(&settings)?;

            let engine = Engine::new(config)?;
            refresh_upstream_index(&settings.upstream_mcp, &engine.store).await?;
            let tools = engine
                .store
                .list_upstream_tools()?
                .iter()
                .filter(|t| t.server.eq_ignore_ascii_case(name))
                .count();
            if tools == 0 {
                println!("{verb} '{name}': 0 tools indexed. Run `agent-brain mcp test {name}` to see why.");
            } else {
                println!("{verb} '{name}': {tools} tools indexed.");
            }
        }
        "remove" | "rm" => {
            let name = args
                .get(3)
                .context("usage: agent-brain mcp remove <name>")?;
            let len_before = settings.upstream_mcp.servers.len();
            settings
                .upstream_mcp
                .servers
                .retain(|s| !s.name.eq_ignore_ascii_case(name));

            if settings.upstream_mcp.servers.len() == len_before {
                println!("No server named '{name}' found.");
                return Ok(());
            }

            persist_brain_settings(&settings)?;

            let engine = Engine::new(config)?;
            let _ = refresh_upstream_index(&settings.upstream_mcp, &engine.store).await;
            println!("Removed MCP server '{name}'.");
        }
        "sync" => {
            let engine = Engine::new(config)?;
            println!("Syncing tool schemas from all enabled MCP servers...");
            let count = refresh_upstream_index(&settings.upstream_mcp, &engine.store).await?;
            println!(
                "Synced {count} tools from {} server(s).",
                settings.upstream_mcp.servers.len()
            );
        }
        "test" => {
            let name = args.get(3).context("usage: agent-brain mcp test <name>")?;
            let server = settings
                .upstream_mcp
                .servers
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
                .context(format!("server '{name}' not found"))?;

            println!(
                "Probing MCP server '{name}' ({} {})...",
                server.command,
                server.args.join(" ")
            );
            let start = Instant::now();
            let tools = list_server_tools(server).await?;
            let elapsed = start.elapsed();

            println!("✓ Connected successfully in {:.2?}", elapsed);
            println!("Discovered {} tool(s):", tools.len());
            for t in tools {
                println!("  - {:<28} {}", t.name, t.description);
            }
        }
        _ => {
            eprintln!("Usage: agent-brain mcp <list|add|remove|sync|test> [args...]");
            eprintln!();
            eprintln!("Commands:");
            eprintln!("  list                          List registered MCP servers and indexed tool counts");
            eprintln!("  add <name> -- <cmd> [args...] Register a new upstream MCP server");
            eprintln!("  remove <name>                 Remove a registered MCP server");
            eprintln!(
                "  sync                          Re-query all servers and refresh tool index"
            );
            eprintln!(
                "  test <name>                   Probe connection and list available tool schemas"
            );
            std::process::exit(1);
        }
    }

    Ok(())
}
