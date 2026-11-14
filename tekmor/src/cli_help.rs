pub fn help_target(args: &[String]) -> Option<Option<&str>> {
    let wants_help = args.iter().any(|a| a == "--help" || a == "-h");
    if !wants_help && args.get(1).map(String::as_str) != Some("help") {
        return None;
    }
    let topic = args
        .iter()
        .skip(1)
        .map(String::as_str)
        .find(|a| !a.starts_with('-') && *a != "help");
    Some(topic)
}

pub fn print_root() {
    println!(
        "tekmor: a proof eval for coding agents\n\n\
Usage:  tekmor <command> [args]\n\n\
Commands:\n\
  verify -- <cmd>          Run an allowlisted check and record its outcome\n\
  verify receipt           Show whether the current commit and code passed\n\
  verify summary           Per check: failed runs out of all runs\n\
  verify init              Create .verify/ with an allowlist\n\
  stop-gate                Stop hook: block a turn that has no passing proof\n\
  bash-gate                PreToolUse hook: check a shell command before it runs\n\
  commit-gate              PreToolUse hook: refuse commits that skip git hooks\n\
  install [--global]       Register the gates in Claude Code settings\n\
  ext list|add|remove      Manage optional extensions\n\n\
Run `tekmor help <command>` for details."
    );
}

pub fn print_command(cmd: &str) -> bool {
    let text = match cmd {
        "verify" => "Usage:  tekmor verify -- <cmd>\n        tekmor verify receipt [--project <DIR>] [--markdown|--json]\n        tekmor verify summary [--json]\n        tekmor verify init [--force] [--dir <DIR>]\n\nRuns an allowlisted command from .verify/verify.toml and records the outcome.\n",
        "receipt" => "Usage:  tekmor verify receipt [--project <DIR>] [--markdown|--json]\n\nExit 0 only when the current commit and code state passed.\n",
        "stop-gate" => "Usage:  tekmor stop-gate\n\nClaude Code Stop hook. Blocks a turn that changed code without a passing run or a feature map.\n",
        "bash-gate" => "Usage:  tekmor bash-gate\n\nClaude Code PreToolUse hook for Bash. Blocks or warns on risky commands.\n",
        "commit-gate" => "Usage:  tekmor commit-gate\n\nRefuses commits that skip git hooks.\n",
        "install" => "Usage:  tekmor install [--global] [--quiet]\n\nRegisters the bash-gate and stop-gate hooks in Claude Code settings.\n",
        "ext" => "Usage:  tekmor ext [list | add <name> | remove <name>]\n\nManage optional extensions.\n",
        _ => return false,
    };
    println!("{text}");
    true
}
