//! Docker-style CLI help: short grouped root + per-command detail.

/// Root help, or `Some(command)` when the user asked for command-specific help.
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
Usage:  tekmor [OPTIONS] COMMAND

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
  config        Init or show ~/.tekmor/config.yaml
  mode          Mode snippet paths / install for hosts
  update        Auto-update the tekmor binary
  secrets       Keychain secret refs for upstream MCP

Memory & Sync:
  export        Export a sync bundle
  import        Import a sync bundle
  sync          Git / cloud sync and conflict restore
  sessions      Ingest or status session digests
  bootstrap     Join this device to a shared sync repo
  memory        Archive / observe / extract memory
  gc            Dedup, prune, and vacuum the DB
  suggest-memory  Review hook anti-pattern suggestions
  promote       Approve or reject staged skills
  recall        Read stored facts as JSON, by tag, query, age, or similarity
  remember      Store one fact with tags and a source
  import-corpus Store the paragraphs of a folder of Markdown files

Ops:
  stats         Local route / latency metrics
  dashboard     Write a local HTML metrics page
  inspect       Retrieval logs, facts, conflicts
  learn         Ingest allowlisted docs into skills + memory
  log           Print or follow a named log file
  verify        Execute allowlisted check and record trusted outcome
  report        Print the verify report by source
  status        Show brain status for the current directory
  rules         Command rules for the Bash gate
  ext           Install or remove an extension
  memorable     Optional Memorable integration
  feature-graph Print the feature-map graph as JSON

Extras (build with `--features extras`):
  dataset       Export trajectories as training JSONL (extras extension)
  digest        Weekly operator digest (extras extension)
  eval          Recall gates (bench extension)
  fixture       Build committed fixture DBs (extras extension)
  skills-sh     Refresh skills.sh snapshot (extras extension)
  proofs        CI proof aggregate
  distill       Architecture distill / clusters (extras extension)
  graphify      Codebase graph (graphify extension: tekmor ext add graphify)
  grpc          Orchestrator bridge API (gRPC, extras extension)

Global Options:
  -h, --help       Show help
  -V, --version    Print version and quit

Run 'tekmor COMMAND --help' for more information on a command.
See README.md for the full guide.
"
    );
}

/// Print detailed help for `cmd`. Returns false if the command is unknown.
pub fn print_command(cmd: &str) -> bool {
    let text = match cmd {
        "recall" => tekmor::recall::USAGE,
        "remember" => tekmor::remember::USAGE,
        "import-corpus" => tekmor::remember::IMPORT_USAGE,
        "report" => "Usage:  tekmor report\n\nPrint the verify report by source.\n",
        "status" => "Usage:  tekmor status\n\nShow brain status for the current directory.\n",
        "bootstrap" => {
            "Usage:  tekmor bootstrap [--remote URL] [--skip-install]\n\nJoin this device to a shared sync repo. Without --remote, uses sync.git.remote from config.yaml.\n"
        }
        "rules" => {
            "Usage:  tekmor rules [list | add KEY=VALUE... | remove ID | reset]\n\nCommand rules for the Bash gate. Run these yourself; agents cannot change rules.\n"
        }
        "ext" => "Usage:  tekmor ext [list | add NAME | remove NAME]\n\nInstall or remove an extension.\n",
        "memorable" => {
            "Usage:  tekmor memorable [status | enable [--capture] | disable | list | capture TRACE.json | capture-transcript ...]\n\nOptional Memorable integration. With --capture, verified sessions are sent to Memorable's extraction service.\n"
        }
        "bash-gate" => "Usage:  tekmor bash-gate\n\nHook entry point for the Bash gate. Host hooks call it; run it by hand only to test.\n",
        "commit-gate" => "Usage:  tekmor commit-gate\n\nHook entry point for the commit gate. Host hooks call it; run it by hand only to test.\n",
        "stop-gate" => "Usage:  tekmor stop-gate\n\nHook entry point for the stop gate. The Stop hook calls it; run it by hand only to test.\n",
        "capabilities-hook" => "Usage:  tekmor capabilities-hook\n\nHook entry point that prints the capability rule at session start. Host hooks call it.\n",
        "serve" => {
            "\
Usage:  tekmor serve

Start the MCP server on stdio.

Cursor (and other hosts) launch this automatically after install —
you usually do not run serve yourself.
"
        }
        "install" => {
            "\
Usage:  tekmor install [OPTIONS]

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
  tekmor install --global
  tekmor install --all --global
  tekmor install --claude-code --global
"
        }
        "doctor" => {
            "\
Usage:  tekmor doctor [OPTIONS]

Check MCP install, binary, hooks, and codesign.

Options:
      --fix       Re-sign binary (macOS), align mcp.json, refresh hooks
"
        }
        "add" => {
            "\
Usage:  tekmor add <@alias|owner/repo|url>...

Install package(s) and index them.

Examples:
  tekmor add @agstack
  tekmor add @supervisor
  tekmor add @starter
  tekmor add @nextjs
  tekmor add affaan-m/ecc
  tekmor add https://github.com/affaan-m/ecc

See also: tekmor registry list
"
        }
        "index" => {
            "\
Usage:  tekmor index [OPTIONS]

Reindex skills, rules, and memory from installed packages + home.

Options:
  -c, --changed-only   Skip files whose mtime is unchanged
      --no-ast         Skip AST symbol extraction
"
        }
        "briefing" => {
            "\
Usage:  tekmor briefing

Print the last human-readable route summary
(~/.tekmor/logs/last-route.md).
"
        }
        "feature-graph" => {
            "\
Usage:  tekmor feature-graph [DIR]

Print the feature maps as a JSON graph of nodes, sub-features, code
paths, and [[wikilink]] edges. DIR defaults to .verify/features.
"
        }
        "onboarding" => {
            "\
Usage:  tekmor onboarding

Print the USP tagline and a 5-minute getting-started checklist.
"
        }
        "version" => {
            "\
Usage:  tekmor version
        tekmor --version

Print the installed tekmor version.
"
        }
        "package" => {
            "\
Usage:  tekmor package COMMAND [OPTIONS]

Manage installed skill and prompt packages.

Commands:
  add <source> [--ref <git_ref>] [--no-index]
                    Install a package (@alias, owner/repo, or GitHub URL)
  list              List all installed packages and their local paths
  update [name]     Update one package, or all if name omitted
  remove <name>     Remove an installed package and purge indexed items

Examples:
  tekmor package add @matt-skills
  tekmor package add anthropics/skills
  tekmor package list
  tekmor package remove matt-skills
"
        }
        "mcp" => {
            "\
Usage:  tekmor mcp COMMAND [OPTIONS]

Manage upstream MCP servers, dynamic tool indexing, and schemas.

Commands:
  list                          List registered MCP servers, statuses, and tool counts
  add <name> -- <cmd> [args...] Register a new upstream MCP server and index its tools
  remove <name>                 Remove a registered MCP server and purge its tools
  sync                          Re-query all servers and refresh vector tool catalog
  test <name>                   Probe connection and list available tool schemas

Examples:
  tekmor mcp add github -- npx -y @modelcontextprotocol/server-github
  tekmor mcp add postgres --env=DATABASE_URL=postgres://... -- npx -y @modelcontextprotocol/server-postgres
  tekmor mcp list
  tekmor mcp test github
  tekmor mcp sync
"
        }
        "registry" => {
            "\
Usage:  tekmor registry COMMAND

Browse and refresh curated @aliases.

Commands:
  list [--kind skill_package|utility|workflow]
                        Show curated aliases (default)
  sync [--local]        Refresh registry cache (remote or embedded)
"
        }
        "config" => {
            "\
Usage:  tekmor config COMMAND

Commands:
  init    Write ~/.tekmor/config.yaml defaults
  show    Print the active config file path / contents
"
        }
        "mode" => {
            "\
Usage:  tekmor mode COMMAND

Commands:
  paths                 List mode files per host
  show                  Print the mode snippet (markdown)
  install [--global]    Install mode files for all hosts
"
        }
        "update" => {
            "\
Usage:  tekmor update [OPTIONS]

Run auto-update (release redirect; API fallback uses GITHUB_TOKEN/GH_TOKEN).

Options:
      --force       Force update even if already current
      --mcp-only    Only refresh MCP / hook wiring
"
        }
        "secrets" => {
            "\
Usage:  tekmor secrets COMMAND

Keychain secret refs for upstream MCP.

Commands:
  status              Show configured secret refs
  setup               Interactive setup for missing secrets
  add <name> <value>  Store a secret in the keychain
"
        }
        "export" => {
            "\
Usage:  tekmor export [dir] [--with-lessons]

Export a sync bundle (manifest + facts.jsonl).
Default dir: ~/.tekmor/export/
"
        }
        "import" => {
            "\
Usage:  tekmor import <dir> [--policy POLICY]

Import a sync bundle.

Options:
      --policy POLICY   Conflict policy (default: newer_wins)
"
        }
        "sync" => {
            "\
Usage:  tekmor sync COMMAND

Git / cloud sync and conflict restore.

Commands:
  status                      Git sync + recent conflicts summary
  restore <conflict-id>       Re-promote a fact from conflict_log
  git init [--remote URL]     Init ~/.tekmor/sync git repo
  git clone [--remote URL]    Clone sync repo on a second machine
  git push                    Export bundle, commit, push
  git pull                    Pull and import bundle
  git status                  Show git sync repo state
  cloud push|pull             Encrypted cloud sync (S3 / local)
"
        }
        "sessions" => {
            "\
Usage:  tekmor sessions COMMAND

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
Usage:  tekmor memory COMMAND

Commands:
  gc [--apply] [--force] [--stale-days N] [--very-stale-days N]
      Archive stale facts (dry-run by default; pass --apply to write)
  observe
      Run observation pass over recent activity
  extract
      Explain / extract pending traces

See also: tekmor gc
"
        }
        "gc" => {
            "\
Usage:  tekmor gc [OPTIONS]

Run DB garbage collection: dedup, prune, vacuum.

Options:
      --min-confidence N   Drop facts below this confidence
      --max-age-days N     Age hint for prune (reserved)

See also: tekmor memory gc
"
        }
        "suggest-memory" => {
            "\
Usage:  tekmor suggest-memory [approve|reject]

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
Usage:  tekmor promote COMMAND

Skill promotion workflow (human approval required).

Commands:
  list [--status pending|approved|rejected]
  approve <staging-id>
  reject <staging-id>
"
        }
        "stats" => {
            "\
Usage:  tekmor stats [OPTIONS]

Local index / routing / latency metrics (not product ROI claims).

Options:
      --days N    Lookback window (default: 7)
      --json      Machine-readable JSON
      --markdown  Shareable summary of measured figures only, for a weekly update
"
        }
        "dashboard" => {
            "\
Usage:  tekmor dashboard [OPTIONS]

Write a local HTML metrics page (routes, latency, activity).

Options:
      --days N    Lookback window (default: 7)
      --open      Open the HTML file in the default browser
"
        }
        "inspect" => {
            "\
Usage:  tekmor inspect COMMAND

Commands:
  log [--last] [--limit N]   List retrieval logs (route_task output)
  fact <id>                  Show a stored memory fact
  conflicts [--limit N]      Topic supersession conflict log
"
        }
        "learn" => {
            "\
Usage:  tekmor learn COMMAND

Ingest allowlisted docs into skills + memory.

Commands:
  url <https-url> [--topic NAME] [--dry-run]
  allowlist | domains
      Show docs.allowed_domains from config
"
        }
        "log" => {
            "\
Usage:  tekmor log <name> [--follow]
        tekmor log --list

Print or follow a named log under the tekmor home.

Options:
      --follow    Tail the log
      --list      List available logs
"
        }
        "verify" => {
            "\
Usage:  tekmor verify [OPTIONS] -- <cmd> [args...]
        tekmor verify init [--force] [--dir <DIR>]
        tekmor verify record --exit N [OPTIONS] -- <cmd summary...>
        tekmor verify run [--project <DIR>] [--json]
        tekmor verify receipt [--project <DIR>] [--markdown|--json]
        tekmor verify summary [--json]

Run an allowlisted verification command and record trusted outcome.

Subcommands:
  init                  Initialize .verify/ directory with verify.toml and run.sh template
  record                Record already-known exit code without executing
  run                   Run the [verify.default] step declared in verify.toml, with its timeout
  receipt               Show whether the current commit and code state passed its verify run.
                        Exit 0 only when PASSED. --markdown prints a block to paste into a PR.
  summary               Per check: failed runs out of all runs, and the latest exit code.
                        Failing checks come first. --json prints the same as JSON.

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
Provided by the bench extension (`tekmor ext add bench`).
Options:
  --ci [--live]              Recall@3 gate
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
        "grpc" => "Usage: tekmor grpc serve [--addr HOST:PORT]\n",
        "distill" => {
            "\
Usage:
  tekmor distill [--out PATH]
  tekmor distill clusters [--threshold N] [--dry-run]
"
        }
        _ => "",
    };
    eprintln!(
        "\
Usage:  tekmor {cmd} ...

Extras command — requires a build with `--features extras`.

{detail}Rebuild:
  cargo install --path tekmor --features extras
  cargo run -p tekmor --features extras -- {cmd} --help
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(xs: &[&str]) -> Vec<String> {
        std::iter::once("tekmor")
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
            "serve",
            "install",
            "doctor",
            "add",
            "sync",
            "eval",
            "dataset",
            "verify",
            "report",
            "status",
            "bootstrap",
            "rules",
            "ext",
            "memorable",
            "bash-gate",
            "stop-gate",
        ] {
            assert!(print_command(cmd), "{cmd}");
        }
        assert!(!print_command("not-a-real-command"));
    }
}
