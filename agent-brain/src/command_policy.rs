use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::command_rules::{self, Rule};
use crate::commit_gate::{
    base_name, bypasses_git_hooks, command_start, shell_payload, split_segments, substitutions,
};

const SCRATCH_ROOTS: &[&str] = &["/tmp", "/private/tmp", "/var/folders"];
const SYSTEM_ROOTS: &[&str] = &[
    "/etc",
    "/private/etc",
    "/System",
    "/usr",
    "/bin",
    "/sbin",
    "/Library",
    "/dev",
];
const HARMLESS_DEVICES: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty"];
const PROTECTED_HOME: &[&str] = &[
    ".claude",
    ".ssh",
    ".autonomic",
    ".agent_brain",
    ".config",
    ".gitconfig",
    ".zshrc",
    ".bashrc",
    ".profile",
];
const GIT_REDIRECTS: &[&str] = &[
    "GIT_DIR=",
    "GIT_WORK_TREE=",
    "GIT_INDEX_FILE=",
    "GIT_OBJECT_DIRECTORY=",
    "GIT_CONFIG",
];
const INTERPRETERS: &[&str] = &[
    "perl",
    "ruby",
    "node",
    "python",
    "python3",
    "bun",
    "deno",
    "php",
    "lua",
    "osascript",
];
const DESTRUCTIVE_CODE: &[&str] = &[
    "unlink",
    "rmtree",
    "remove(",
    "rmdir",
    "rmSync",
    "rmdirSync",
    "unlinkSync",
    "rimraf",
    "rm_rf",
    "FileUtils",
    "os.system",
    "system(",
    "exec(",
    "subprocess",
    "child_process",
    "execSync",
    "execFile",
    "spawnSync",
    "shutil",
    "truncate",
    "open(",
    "writeFile",
    "write_text",
    "os.remove",
    "File.write",
    "File.delete",
    "removeSync",
    "do shell script",
];
const DESTRUCTIVE_GIT: &[&str] = &[
    "clean", "reset", "checkout", "restore", "push", "stash", "rm",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Warn(String),
    Block(String),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BashInput {
    cwd: Option<String>,
    tool_input: BashToolInput,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BashToolInput {
    command: String,
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    root: Option<&'a Path>,
    rules: &'a [Rule],
    piped: bool,
}

pub fn classify(command: &str, root: Option<&Path>, rules: &[Rule]) -> Verdict {
    let ctx = Ctx {
        root,
        rules,
        piped: false,
    };
    classify_in(command, ctx, root.map(normalize))
}

fn classify_in(command: &str, ctx: Ctx<'_>, mut dir: Option<PathBuf>) -> Verdict {
    if bypasses_git_hooks(command) {
        return Verdict::Block("it skips git hooks; commits must run the pre-commit guard".into());
    }
    let ctx = Ctx {
        piped: command.contains('|') || command.contains("<("),
        ..ctx
    };
    let mut verdict = Verdict::Allow;
    for segment in split_segments(command) {
        let words: Vec<&str> = segment.iter().map(String::as_str).collect();
        let start = command_start(&words);
        let step = segment_verdict(&words[..start], &words[start..], ctx, &mut dir);
        verdict = merge(verdict, step);
    }
    for target in redirect_targets(command) {
        for base in [ctx.root, dir.as_deref()] {
            verdict = merge(verdict, write_verdict(&target, ctx.root, base));
        }
    }
    for inner in substitutions(command) {
        verdict = merge(verdict, classify_in(inner, ctx, dir.clone()));
    }
    verdict
}

fn segment_verdict(
    prefix: &[&str],
    argv: &[&str],
    ctx: Ctx<'_>,
    dir: &mut Option<PathBuf>,
) -> Verdict {
    if prefix.iter().any(|assignment| {
        GIT_REDIRECTS
            .iter()
            .any(|name| assignment.starts_with(name))
    }) {
        return Verdict::Block("it redirects git's repository, index, or config".into());
    }
    let Some((&first, args)) = argv.split_first() else {
        return Verdict::Allow;
    };
    if first.starts_with(['>', '<']) || first.chars().all(|c| c.is_ascii_digit()) {
        return Verdict::Allow;
    }
    if first.contains(['$', '`', '(']) {
        return Verdict::Block(format!("the command name is computed at runtime: {first}"));
    }
    let root = ctx.root;
    match base_name(first) {
        "cd" | "pushd" => {
            *dir = change_dir(dir.as_deref(), args);
            Verdict::Allow
        }
        "git" => git_verdict(args, root, dir.as_deref()),
        "find" => find_verdict(args, ctx, dir.as_deref()),
        "eval" => {
            let code = args.join(" ");
            if code.contains(['$', '`']) {
                Verdict::Block("eval runs code that is computed at runtime".into())
            } else {
                classify_in(&code, ctx, dir.clone())
            }
        }
        "sh" | "bash" | "zsh" | "dash" | "ksh" => shell_verdict(args, ctx, dir.as_deref()),
        "source" | "." => Verdict::Warn(format!("sources code that cannot be inspected: {first}")),
        "dd" if args.iter().any(|arg| is_device_write(arg)) => {
            Verdict::Block("dd writes to a device".into())
        }
        "agent-brain" if matches!(args, ["rules", "add" | "remove" | "reset", ..]) => {
            Verdict::Block("rule changes are made by the user, not by the agent".into())
        }
        name if name.starts_with("mkfs") => Verdict::Block("mkfs formats a filesystem".into()),
        name if INTERPRETERS.contains(&name) => merge(
            interpreter_verdict(args),
            rule_verdict(name, args, ctx, dir.as_deref(), false),
        ),
        name => rule_verdict(name, args, ctx, dir.as_deref(), true),
    }
}

fn rule_verdict(
    name: &str,
    args: &[&str],
    ctx: Ctx<'_>,
    dir: Option<&Path>,
    warn_unknown: bool,
) -> Verdict {
    let rules: Vec<&Rule> = ctx
        .rules
        .iter()
        .filter(|rule| rule.command == name)
        .collect();
    if rules.is_empty() {
        return if warn_unknown {
            Verdict::Warn(format!(
                "unrecognised command {name}; allow it or add a rule with agent-brain rules add"
            ))
        } else {
            Verdict::Allow
        };
    }
    rules.iter().fold(Verdict::Allow, |verdict, rule| {
        merge(verdict, apply_rule(rule, args, ctx.root, dir))
    })
}

fn named_by_rule(rule: &Rule, reason: String) -> String {
    format!(
        "{reason} (rule {id}; remove it with `agent-brain rules remove {id}`)",
        id = rule.id
    )
}

fn apply_rule(rule: &Rule, args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    if !command_rules::is_triggered(&rule.trigger, args) {
        return Verdict::Allow;
    }
    let reason = if rule.reason.is_empty() {
        format!("blocked by the rule for {}", rule.command)
    } else {
        rule.reason.clone()
    };
    match rule.mode.as_str() {
        "allow" => Verdict::Allow,
        "block" => Verdict::Block(named_by_rule(rule, reason)),
        "warn" => Verdict::Warn(named_by_rule(rule, reason)),
        "operands" => {
            let value_flags = command_rules::value_flag_list(rule);
            let recursive = !rule.recursive_on.is_empty()
                && command_rules::is_triggered(&rule.recursive_on, args);
            let targets = command_rules::select_targets(&rule.targets, args, &value_flags);
            if rule.role == "delete" && recursive && targets.is_empty() {
                return Verdict::Block("recursive delete without explicit targets".into());
            }
            worst(&targets, |target| {
                if rule.role == "write" {
                    write_verdict(target, root, dir)
                } else {
                    delete_verdict(target, recursive, root, dir)
                }
            })
        }
        other => Verdict::Block(format!("unknown rule mode {other}")),
    }
}

fn shell_verdict(args: &[&str], ctx: Ctx<'_>, dir: Option<&Path>) -> Verdict {
    match shell_payload(args) {
        Some(payload) => classify_in(payload, ctx, dir.map(Path::to_path_buf)),
        None if ctx.piped => Verdict::Block("it runs code piped into a shell".into()),
        None => Verdict::Warn("runs a script file that cannot be inspected".into()),
    }
}

fn quote_words(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| {
            if word.is_empty() || word.contains(char::is_whitespace) {
                format!("'{word}'")
            } else {
                (*word).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_device_write(arg: &str) -> bool {
    arg.strip_prefix("of=")
        .is_some_and(|target| target.starts_with("/dev/") && !HARMLESS_DEVICES.contains(&target))
}

fn merge(current: Verdict, next: Verdict) -> Verdict {
    match (&current, &next) {
        (Verdict::Block(_), _) => current,
        (_, Verdict::Block(_)) => next,
        (Verdict::Warn(_), _) => current,
        _ => next,
    }
}

fn worst(targets: &[&str], check: impl Fn(&str) -> Verdict) -> Verdict {
    targets.iter().fold(Verdict::Allow, |verdict, target| {
        merge(verdict, check(target))
    })
}

fn change_dir(current: Option<&Path>, args: &[&str]) -> Option<PathBuf> {
    match args.first() {
        None => home_dir(),
        Some(&"-") => None,
        Some(&target) => resolve(target, current),
    }
}

fn resolve(target: &str, dir: Option<&Path>) -> Option<PathBuf> {
    let path = expand_path(target)?;
    if path.is_absolute() {
        Some(normalize(&path))
    } else {
        dir.map(|dir| normalize(&dir.join(path)))
    }
}

fn expand_path(target: &str) -> Option<PathBuf> {
    let home_rest = target
        .strip_prefix("$HOME")
        .or_else(|| target.strip_prefix("${HOME}"));
    if let Some(rest) = home_rest.filter(|rest| rest.is_empty() || rest.starts_with('/')) {
        return home_dir().map(|home| home.join(rest.trim_start_matches('/')));
    }
    if target.contains(['$', '`', '*', '?', '[']) {
        return None;
    }
    if let Some(rest) = target.strip_prefix('~') {
        if !rest.is_empty() && !rest.starts_with('/') {
            return None;
        }
        return home_dir().map(|home| home.join(rest.trim_start_matches('/')));
    }
    Some(PathBuf::from(target))
}

fn delete_verdict(
    target: &str,
    recursive: bool,
    root: Option<&Path>,
    dir: Option<&Path>,
) -> Verdict {
    let Some(path) = resolve(target, dir) else {
        return if recursive {
            Verdict::Block(format!("recursive delete of an unresolved path: {target}"))
        } else {
            Verdict::Warn(format!("delete of an unresolved path: {target}"))
        };
    };
    let git_metadata = path.components().any(|part| part.as_os_str() == ".git");
    if git_metadata || is_protected(&path) || is_root_or_home(&path) || in_system_root(&path) {
        return Verdict::Block(format!(
            "would delete a git, protected, system, root, or home path: {target}"
        ));
    }
    let inside_project = root.is_some_and(|root| path != root && path.starts_with(root));
    if inside_project || is_scratch(&path) {
        return if recursive {
            Verdict::Warn(format!(
                "recursive delete inside the project or scratch: {target}"
            ))
        } else {
            Verdict::Allow
        };
    }
    if recursive {
        Verdict::Block(format!("recursive delete outside the project: {target}"))
    } else {
        Verdict::Warn(format!("delete outside the project: {target}"))
    }
}

fn write_verdict(target: &str, root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    if HARMLESS_DEVICES.contains(&target) {
        return Verdict::Allow;
    }
    if target.starts_with("/dev/") {
        return Verdict::Block(format!("writes to a device: {target}"));
    }
    let Some(path) = resolve(target, dir) else {
        return Verdict::Warn(format!("write to an unresolved path: {target}"));
    };
    if path == Path::new("/") || is_protected(&path) || in_system_root(&path) {
        return Verdict::Block(format!("writes a protected or system path: {target}"));
    }
    if root.is_some_and(|root| path.starts_with(root)) || is_scratch(&path) {
        return Verdict::Allow;
    }
    Verdict::Warn(format!("write outside the project: {target}"))
}

fn redirect_targets(command: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = command.char_indices();
    while let Some((index, ch)) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, '\\') => {
                chars.next();
            }
            (None, '>') => {
                let mut rest = &command[index + 1..];
                if rest.starts_with('(') || rest.starts_with('&') {
                    continue;
                }
                rest = rest.strip_prefix('|').unwrap_or(rest);
                if let Some(after) = rest.strip_prefix('>') {
                    rest = after;
                    chars.next();
                }
                let word: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace() && !";&|()<>".contains(*c))
                    .collect();
                let word = word.trim_matches(|c| c == '"' || c == '\'');
                if !word.is_empty() {
                    targets.push(word.to_string());
                }
            }
            (None, _) => {}
        }
    }
    targets
}

fn git_verdict(args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    let mut index = 0;
    let mut outside_repo = None;
    while let Some(&token) = args.get(index) {
        let (config, step) = match token {
            "-c" | "--config-env" => (args.get(index + 1).copied(), 2),
            "-C" => {
                if let Some(&target) = args.get(index + 1) {
                    let inside = resolve(target, dir)
                        .is_some_and(|path| root.is_some_and(|root| path.starts_with(root)));
                    if !inside {
                        outside_repo = Some(target);
                    }
                }
                (None, 2)
            }
            "--namespace" => (None, 2),
            "--git-dir" | "--work-tree" => {
                return Verdict::Block("it redirects git's repository or working tree".into());
            }
            _ if token.starts_with("--git-dir=") || token.starts_with("--work-tree=") => {
                return Verdict::Block("it redirects git's repository or working tree".into());
            }
            _ if token.starts_with("--config-env=") => (Some(&token["--config-env=".len()..]), 1),
            _ if token.starts_with("-c") => (Some(&token[2..]), 1),
            _ if token.starts_with('-') => (None, 1),
            _ => break,
        };
        if config.is_some_and(is_redirecting_config) {
            return Verdict::Block(
                "it changes git aliases, the hook path, or config includes".into(),
            );
        }
        index += step;
    }
    let Some(&subcommand) = args.get(index) else {
        return Verdict::Allow;
    };
    let rest = &args[index + 1..];
    if let Some(target) = outside_repo {
        return if DESTRUCTIVE_GIT.contains(&subcommand) {
            Verdict::Block(format!(
                "git -C targets a path outside the project: {target}"
            ))
        } else {
            Verdict::Warn(format!(
                "git -C targets a path outside the project: {target}"
            ))
        };
    }
    match subcommand {
        "config" if rest.iter().any(|arg| is_redirecting_config(arg)) => {
            Verdict::Block("it changes git aliases, the hook path, or config includes".into())
        }
        "push" if rest.iter().any(|arg| is_force_push(arg)) => {
            Verdict::Block("a force push rewrites remote history".into())
        }
        "reset" if rest.contains(&"--hard") => {
            Verdict::Warn("git reset --hard discards uncommitted changes".into())
        }
        "clean" if rest.iter().any(|arg| is_forced_clean(arg)) => {
            Verdict::Warn("git clean deletes untracked files".into())
        }
        "checkout" | "restore" if rest.contains(&".") && !rest.contains(&"--staged") => {
            Verdict::Warn("git checkout or restore of . discards uncommitted changes".into())
        }
        "stash" if matches!(rest.first(), Some(&("drop" | "clear"))) => {
            Verdict::Warn("git stash drop or clear discards stashed changes".into())
        }
        _ => Verdict::Allow,
    }
}

fn is_redirecting_config(config: &str) -> bool {
    let lower = config.to_ascii_lowercase();
    lower.starts_with("core.hookspath")
        || lower.starts_with("core.worktree")
        || lower.starts_with("alias.")
        || lower.starts_with("include.")
}

fn has_short_flag(arg: &str, flag: char) -> bool {
    arg.starts_with('-') && !arg.starts_with("--") && arg.contains(flag)
}

fn is_force_push(arg: &str) -> bool {
    arg.starts_with("--force")
        || arg == "--mirror"
        || arg.starts_with('+')
        || has_short_flag(arg, 'f')
}

fn is_forced_clean(arg: &str) -> bool {
    arg == "--force" || has_short_flag(arg, 'f')
}

fn find_verdict(args: &[&str], ctx: Ctx<'_>, dir: Option<&Path>) -> Verdict {
    let mut verdict = Verdict::Allow;
    for (index, arg) in args.iter().enumerate() {
        if !matches!(*arg, "-exec" | "-execdir" | "-ok") {
            continue;
        }
        let inner: Vec<&str> = args[index + 1..]
            .iter()
            .copied()
            .take_while(|arg| *arg != ";" && *arg != "+")
            .collect();
        if !inner.is_empty() {
            verdict = merge(
                verdict,
                classify_in(&quote_words(&inner), ctx, dir.map(Path::to_path_buf)),
            );
        }
    }
    if !args.contains(&"-delete") {
        return verdict;
    }
    let starts: Vec<&str> = args
        .iter()
        .copied()
        .take_while(|arg| !arg.starts_with('-') && *arg != "(" && *arg != "!")
        .collect();
    let starts = if starts.is_empty() { vec!["."] } else { starts };
    let root = ctx.root;
    let deletes = worst(&starts, |start| {
        if resolve(start, dir).as_deref() == root {
            Verdict::Warn(format!("find -delete at the project root: {start}"))
        } else {
            delete_verdict(start, true, root, dir)
        }
    });
    merge(verdict, deletes)
}

fn interpreter_verdict(args: &[&str]) -> Verdict {
    let Some(position) = args
        .iter()
        .position(|arg| matches!(*arg, "-e" | "-c" | "--eval" | "-p" | "-r"))
    else {
        return Verdict::Allow;
    };
    let Some(&code) = args.get(position + 1) else {
        return Verdict::Allow;
    };
    if !DESTRUCTIVE_CODE
        .iter()
        .any(|keyword| code.contains(keyword))
    {
        return Verdict::Allow;
    }
    if has_absolute_path(code) {
        Verdict::Block(
            "interpreter code deletes or spawns deletes at an absolute or home path".into(),
        )
    } else {
        Verdict::Warn("interpreter code deletes or modifies files".into())
    }
}

fn has_absolute_path(code: &str) -> bool {
    code.contains('~')
        || code.contains("$HOME")
        || code.contains("..")
        || code
            .split(|c: char| " '\"(,=`".contains(c))
            .any(|token| token.starts_with('/'))
}

fn is_scratch(path: &Path) -> bool {
    SCRATCH_ROOTS
        .iter()
        .any(|root| path.starts_with(root) && path != Path::new(root))
}

fn in_system_root(path: &Path) -> bool {
    SYSTEM_ROOTS.iter().any(|root| path.starts_with(root))
}

fn is_protected(path: &Path) -> bool {
    home_dir().is_some_and(|home| {
        let home = normalize(&home);
        PROTECTED_HOME
            .iter()
            .any(|name| path.starts_with(home.join(name)))
    })
}

fn is_root_or_home(path: &Path) -> bool {
    path == Path::new("/") || home_dir().is_some_and(|home| normalize(&home) == path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn run_bash_gate() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: BashInput = serde_json::from_str(&raw).unwrap_or_default();
    let root = input
        .cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let rules = command_rules::load(&command_rules::default_path())
        .unwrap_or_else(|_| command_rules::defaults());
    match classify(&input.tool_input.command, root.as_deref(), &rules) {
        Verdict::Allow => {}
        Verdict::Warn(reason) => {
            record_gate_event("bash-warn", &reason);
            println!(
                "{}",
                json!({ "systemMessage": format!("agent-brain warning: {reason}") })
            )
        }
        Verdict::Block(reason) => {
            record_gate_event("bash-block", &reason);
            println!(
                "{}",
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": format!(
                            "agent-brain blocked this command: {reason}. Run it inside the project, or ask the user to run it."
                        )
                    }
                })
            )
        }
    }
    Ok(())
}

fn record_gate_event(kind: &str, reason: &str) {
    let Ok(config) = crate::config::Config::load() else {
        return;
    };
    if let Err(err) =
        crate::gate_events::record(&config.logs_dir.join("gate-events.jsonl"), kind, reason)
    {
        tracing::warn!(error = %err, "gate event write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/work/repo";
    const CASES: &str = include_str!("command_policy_cases.txt");

    #[test]
    fn verdicts_match_the_case_table() {
        let rules = command_rules::defaults();
        let failures: Vec<String> = CASES
            .lines()
            .filter_map(|line| line.split_once(' '))
            .filter_map(|(want, command)| {
                let got = match classify(command, Some(Path::new(ROOT)), &rules) {
                    Verdict::Allow => "allow",
                    Verdict::Warn(_) => "warn",
                    Verdict::Block(_) => "block",
                };
                (got != want).then(|| format!("want {want}, got {got}: {command}"))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn falls_back_to_block_without_a_known_project_root() {
        assert!(matches!(
            classify("rm -rf build", None, &command_rules::defaults()),
            Verdict::Block(_)
        ));
    }

    #[test]
    fn rule_block_names_the_matching_rule_and_how_to_change_it() {
        let mut rule = Rule::new("docker", "block");
        rule.id = 7;
        rule.reason = "no containers".into();
        match classify("docker run alpine", Some(Path::new(ROOT)), &[rule]) {
            Verdict::Block(reason) => {
                assert!(reason.contains("no containers"), "{reason}");
                assert!(reason.contains("rule 7"), "{reason}");
                assert!(reason.contains("agent-brain rules remove 7"), "{reason}");
            }
            _ => panic!("expected the rule to block"),
        }
    }
}
