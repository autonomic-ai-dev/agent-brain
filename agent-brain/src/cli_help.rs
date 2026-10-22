//! Docker-style CLI help: short grouped root + per-command detail.

/// Root help, or `Some(command)` when the user asked for command-specific help.
/// `None` means this invocation is not a help request (e.g. bare `agent-brain` → serve).
pub fn help_target(args: &[String]) -> Option<Option<&str>> {
    match args.get(1).map(String::as_str) {
        Some("help") | Some("--help") | Some("-h") => Some(args.get(2).map(String::as_str)),
        Some(cmd) if args.iter().skip(2).any(|a| a == "--help" || a == "-h") => Some(Some(cmd)),
        _ => None,
    }
}

pub fn print_root() {
    eprintln!(
        "\
Usage:  agent-brain [OPTIONS] COMMAND

A local MCP router for agents, skills, rules, and memory.

Common Commands:
  serve         Start MCP server (stdio)
  install       Configure MCP + hooks for an editor
  doctor        Check install, binary, hooks, codesign
  package add   Install a package (@alias, owner/repo, or URL)
  mcp           Manage and test upstream MCP servers
  index         Reindex skills, rules, and memory
  briefing      Print the last human-readable route summary
  onboarding    5-minute getting-started checklist
  version       Print installed version

Package & MCP Commands:
  package       Manage installed packages (add, list, update, remove)
  mcp           Manage upstream MCP servers (add, list, remove, sync, test)
  registry      List or sync curated @aliases

Config Commands:
  config        Init or show ~/.agent_brain/config.yaml
  mode          Mode snippet paths / install for hosts
  update        Auto-update the agent-brain binary
  secrets       Keychain secret refs for upstream MCP

Memory & Sync:
  export        Export a sync bundle
  import        Import a sync bundle
  sync          Git / cloud sync and conflict restore
  sessions      Ingest or status session digests
  memory        Archive / observe / extract memory
  gc            Dedup, prune, and vacuum the DB
  suggest-memory  Review hook anti-pattern suggestions
  promote       Approve or reject staged skills

Ops:
  stats         Local route / latency metrics
  dashboard     Write a local HTML metrics page
  inspect       Retrieval logs, facts, conflicts
  learn         Ingest allowlisted docs into skills + memory
  log           Print or follow a named log file
  verify        Execute allowlisted check and record trusted outcome

Extras (build with `--features extras`):
  dataset       Export trajectories as training JSONL
  digest        Weekly operator digest
  eval          Recall gates (fixture / skills.sh / beam)
  fixture       Build committed fixture DBs
  skills-sh     Refresh skills.sh snapshot
  proofs        CI proof aggregate
  distill       Architecture distill / clusters
  graphify      Codebase graph enable / ingest / query
  grpc          Orchestrator bridge API (gRPC)

Global Options:
  -h, --help       Show help
  -V, --version    Print version and quit

Run 'agent-brain COMMAND --help' for more information on a command.
See docs/USAGE.md for the full guide.
"
    );
}

/// Print detailed help for `cmd`. Returns false if the command is unknown.
pub fn print_command(cmd: &str) -> bool {
    let text = match cmd {
        "serve" => {
            "\
Usage:  agent-brain serve

Start the MCP server on stdio.

Cursor (and other hosts) launch this automatically after install —
you usually do not run serve yourself.
"
        }
        "install" => {
            "\
Usage:  agent-brain install [OPTIONS]

Configure MCP + hooks for an editor host.

Options:
      --global              User/global scope (where supported)
      --reload              Refresh Cursor MCP entry after install
      --claude-desktop      Claude Desktop MCP config
      --vscode              VS Code mcp.json (workspace or --global)
      --claude-code         Claude Code .mcp.json / ~/.claude.json
      --opencode            OpenCode opencode.json
      --codex               Codex config.toml + hooks.json
      --gemini              Gemini CLI settings.json
      --antigravity         Antigravity mcp_config.json
      --all                 All of the above (user/global scope)
      --starter             Also install the @starter pack

Examples:
  agent-brain install --global
  agent-brain install --all --global
  agent-brain install --claude-code --global
"
        }
        "doctor" => {
            "\
Usage:  agent-brain doctor [OPTIONS]

Check MCP install, binary, hooks, and codesign.

Options:
      --fix       Re-sign binary (macOS), align mcp.json, refresh hooks
      --mcp        Audit agent-body gateway schemas (tools/list)
"
        }
        "add" => {
            "\
Usage:  agent-brain add <@alias|owner/repo|url>...

Install package(s) and index them.

Examples:
  agent-brain add @agstack
  agent-brain add @supervisor
  agent-brain add @starter
  agent-brain add @nextjs
  agent-brain add affaan-m/ecc
  agent-brain add https://github.com/affaan-m/ecc

See also: agent-brain registry list
"
        }
        "index" => {
            "\
Usage:  agent-brain index [OPTIONS]

Reindex skills, rules, and memory from installed packages + home.

Options:
  -c, --changed-only   Skip files whose mtime is unchanged
      --no-ast         Skip AST symbol extraction
"
        }
        "briefing" => {
            "\
Usage:  agent-brain briefing

Print the last human-readable route summary
(~/.agent_brain/logs/last-route.md).
"
        }
        "onboarding" => {
            "\
Usage:  agent-brain onboarding

Print the USP tagline and a 5-minute getting-started checklist.
"
        }
        "version" => {
            "\
Usage:  agent-brain version
        agent-brain --version

Print the installed agent-brain version.
"
        }
        "package" => {
            "\
Usage:  agent-brain package COMMAND [OPTIONS]

Manage installed skill and prompt packages.

Commands:
  add <source> [--ref <git_ref>] [--no-index]
                    Install a package (@alias, owner/repo, or GitHub URL)
  list              List all installed packages and their local paths
  update [name]     Update one package, or all if name omitted
  remove <name>     Remove an installed package and purge indexed items

Examples:
  agent-brain package add @matt-skills
  agent-brain package add anthropics/skills
  agent-brain package list
  agent-brain package remove matt-skills
"
        }
        "mcp" => {
            "\
Usage:  agent-brain mcp COMMAND [OPTIONS]

Manage upstream MCP servers, dynamic tool indexing, and schemas.

Commands:
  list                          List registered MCP servers, statuses, and tool counts
  add <name> -- <cmd> [args...] Register a new upstream MCP server and index its tools
  remove <name>                 Remove a registered MCP server and purge its tools
  sync                          Re-query all servers and refresh vector tool catalog
  test <name>                   Probe connection and list available tool schemas

Examples:
  agent-brain mcp add github -- npx -y @modelcontextprotocol/server-github
  agent-brain mcp add postgres --env=DATABASE_URL=postgres://... -- npx -y @modelcontextprotocol/server-postgres
  agent-brain mcp list
  agent-brain mcp test github
  agent-brain mcp sync
"
        }
        "registry" => {
            "\
Usage:  agent-brain registry COMMAND

Browse and refresh curated @aliases.

Commands:
  list [--kind skill_package|utility|workflow]
                        Show curated aliases (default)
  sync [--local]        Refresh registry cache (remote or embedded)
"
        }
        "config" => {
            "\
Usage:  agent-brain config COMMAND

Commands:
  init    Write ~/.agent_brain/config.yaml defaults
  show    Print the active config file path / contents
"
        }
        "mode" => {
            "\
Usage:  agent-brain mode COMMAND

Commands:
  paths                 List mode files per host
  show                  Print the mode snippet (markdown)
  install [--global]    Install mode files for all hosts
"
        }
        "update" => {
            "\
Usage:  agent-brain update [OPTIONS]

Run auto-update (release redirect; API fallback uses GITHUB_TOKEN/GH_TOKEN).

Options:
      --force       Force update even if already current
      --mcp-only    Only refresh MCP / hook wiring
"
        }
        "secrets" => {
            "\
Usage:  agent-brain secrets COMMAND

Keychain secret refs for upstream MCP.

Commands:
  status              Show configured secret refs
  setup               Interactive setup for missing secrets
  add <name> <value>  Store a secret in the keychain
"
        }
        "export" => {
            "\
Usage:  agent-brain export [dir]

Export a sync bundle (manifest + facts.jsonl).
Default dir: ~/.agent_brain/export/
"
        }
        "import" => {
            "\
Usage:  agent-brain import <dir> [--policy POLICY]

Import a sync bundle.

Options:
      --policy POLICY   Conflict policy (default: newer_wins)
"
        }
        "sync" => {
            "\
Usage:  agent-brain sync COMMAND

Git / cloud sync and conflict restore.

Commands:
  status                      Git sync + recent conflicts summary
  restore <conflict-id>       Re-promote a fact from conflict_log
  git init [--remote URL]     Init ~/.agent_brain/sync git repo
  git clone [--remote URL]    Clone sync repo on a second machine
  git push                    Export bundle, commit, push
  git pull                    Pull and import bundle
  git status                  Show git sync repo state
  cloud push|pull             Encrypted cloud sync (S3 / local)
"
        }
        "sessions" => {
            "\
Usage:  agent-brain sessions COMMAND

Commands:
  ingest [--source SRCS] [--legacy]
      Import session digests (cursor,codex,gemini,opencode)
  status
      Discoverable vs stored session digests

Options:
      --source SRCS   Comma-separated sources (default: all known)
      --legacy        Include legacy digest paths
"
        }
        "memory" => {
            "\
Usage:  agent-brain memory COMMAND

Commands:
  gc [--apply] [--force] [--stale-days N] [--very-stale-days N]
      Archive stale facts (dry-run by default; pass --apply to write)
  observe
      Run observation pass over recent activity
  extract
      Explain / extract pending traces

See also: agent-brain gc
"
        }
        "gc" => {
            "\
Usage:  agent-brain gc [OPTIONS]

Run DB garbage collection: dedup, prune, vacuum.

Options:
      --min-confidence N   Drop facts below this confidence
      --max-age-days N     Age hint for prune (reserved)

See also: agent-brain memory gc
"
        }
        "suggest-memory" => {
            "\
Usage:  agent-brain suggest-memory [approve|reject]

Show or promote a pending hook anti-pattern suggestion
into store_memory.

Commands:
  (default)   Show pending suggestion
  approve     Commit suggestion as memory
  reject      Discard pending suggestion
"
        }
        "promote" => {
            "\
Usage:  agent-brain promote COMMAND

Skill promotion workflow (human approval required).

Commands:
  list [--status pending|approved|rejected]
  approve <staging-id>
  reject <staging-id>
"
        }
        "stats" => {
            "\
Usage:  agent-brain stats [OPTIONS]

Local index / routing / latency metrics (not product ROI claims).

Options:
      --days N    Lookback window (default: 7)
      --json      Machine-readable JSON
"
        }
        "dashboard" => {
            "\
Usage:  agent-brain dashboard [OPTIONS]

Write a local HTML metrics page (routes, latency, activity).

Options:
      --days N    Lookback window (default: 7)
      --open      Open the HTML file in the default browser
"
        }
        "inspect" => {
            "\
Usage:  agent-brain inspect COMMAND

Commands:
  log [--last] [--limit N]   List retrieval logs (route_task output)
  fact <id>                  Show a stored memory fact
  conflicts [--limit N]      Topic supersession conflict log
"
        }
        "learn" => {
            "\
Usage:  agent-brain learn COMMAND

Ingest allowlisted docs into skills + memory.

Commands:
  url <https-url> [--topic NAME] [--dry-run]
  allowlist | domains
      Show docs.allowed_domains from config
"
        }
        "log" => {
            "\
Usage:  agent-brain log <name> [--follow]
        agent-brain log --list

Print or follow a named log under the agent-brain home.

Options:
      --follow    Tail the log
      --list      List available logs
"
        }
        "verify" => {
            "\
Usage:  agent-brain verify [OPTIONS] -- <cmd> [args...]
        agent-brain verify init [--force] [--dir <DIR>]
        agent-brain verify record --exit N [OPTIONS] -- <cmd summary...>
        agent-brain verify run [--project <DIR>] [--json]

Run an allowlisted verification command and record trusted outcome.

Subcommands:
  init                  Initialize .verify/ directory with verify.toml and run.sh template
  record                Record already-known exit code without executing
  run                   Run the [verify.default] step declared in verify.toml, with its timeout

Options:
      --source <SOURCE>    Outcome source: cli (default) or orchestrator (spine/muscle)
      --config <PATH>      Path to config to hash for drift detection
      --scenarios <DIR>    Path to scenario dir (defaults to .verify/ in ancestor)
      --route-log-id <ID>  Route log ID that prompted this verification
      --cause <TAG>        Failure cause tag (defaults to cmd if exit non-zero)
      --force              Overwrite existing .verify files during init
      --dir <DIR>          Target project directory for init (defaults to cwd)

Allowlist:
  Configured in .verify/verify.toml or verify.toml under project root.
  Default fallback matches common test runners (cargo test, pytest, make test, .verify/).
"
        }
        "dataset" | "digest" | "eval" | "bench" | "fixture" | "skills-sh" | "proofs"
        | "distill" | "graphify" | "grpc" => {
            print_extras_command(cmd);
            return true;
        }
        _ => return false,
    };
    eprint!("{text}");
    true
}

fn print_extras_command(cmd: &str) {
    let detail = match cmd {
        "dataset" => {
            "\
Commands:
  export [--out PATH] [--min-confidence N] [--all-outcomes]
  stats
  ingest | pipeline ...
"
        }
        "digest" => "Requires: --weekly\n",
        "eval" => {
            "\
Options:
  --ci [--live]              Recall@3 gate
  --beam                     BEAM memory+routing harness
  --skills-sh [--fixture-db PATH] [--seed] [--write PATH]
"
        }
        "bench" => {
            "\
Options:
  --ci | --mcp | --graphify | --scale | --supervisor | --onnx
  Add --assert (and --write PATH) where supported.
"
        }
        "fixture" => "Commands: build [--index-size N] [--write PATH] ...\n",
        "skills-sh" => "Commands: sync [--max N] [--merge] ...\n",
        "proofs" => "Requires: --ci [--write PATH]\n",
        "graphify" => "Commands: enable|disable|status|ingest|run|query ...\n",
        "grpc" => "Usage: agent-brain grpc serve [--addr HOST:PORT]\n",
        "distill" => {
            "\
Usage:
  agent-brain distill [--out PATH]
  agent-brain distill clusters [--threshold N] [--dry-run]
"
        }
        _ => "",
    };
    eprintln!(
        "\
Usage:  agent-brain {cmd} ...

Extras command — requires a build with `--features extras`.

{detail}Rebuild:
  cargo install --path agent-brain --features extras
  cargo run -p agent-brain --features extras -- {cmd} --help
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(xs: &[&str]) -> Vec<String> {
        std::iter::once("agent-brain")
            .chain(xs.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn root_help_flags() {
        assert_eq!(help_target(&args(&["--help"])), Some(None));
        assert_eq!(help_target(&args(&["-h"])), Some(None));
        assert_eq!(help_target(&args(&["help"])), Some(None));
    }

    #[test]
    fn command_help_via_flag_or_help_subcommand() {
        assert_eq!(
            help_target(&args(&["install", "--help"])),
            Some(Some("install"))
        );
        assert_eq!(help_target(&args(&["help", "sync"])), Some(Some("sync")));
        assert_eq!(help_target(&args(&["serve"])), None);
    }

    #[test]
    fn known_commands_have_detail() {
        for cmd in [
            "serve", "install", "doctor", "add", "sync", "eval", "dataset", "verify",
        ] {
            assert!(print_command(cmd), "{cmd}");
        }
        assert!(!print_command("not-a-real-command"));
    }
}
