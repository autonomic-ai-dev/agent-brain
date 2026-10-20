use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::commit_gate::{
    bypasses_git_hooks, command_start, shell_payload, split_segments, substitutions,
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
    "subprocess",
    "shutil",
    "truncate",
];
const ABSOLUTE_IN_CODE: &[&str] = &["\"/", "'/", "~", "$HOME", ".."];

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

pub fn classify(command: &str, root: Option<&Path>) -> Verdict {
    classify_in(command, root, root.map(normalize))
}

fn classify_in(command: &str, root: Option<&Path>, mut dir: Option<PathBuf>) -> Verdict {
    if bypasses_git_hooks(command) {
        return Verdict::Block("it skips git hooks; commits must run the pre-commit guard".into());
    }
    let mut verdict = Verdict::Allow;
    for segment in split_segments(command) {
        let words: Vec<&str> = segment.iter().map(String::as_str).collect();
        let start = command_start(&words);
        let step = segment_verdict(&words[..start], &words[start..], root, &mut dir);
        verdict = merge(verdict, step);
    }
    for target in redirect_targets(command) {
        for base in [root, dir.as_deref()] {
            verdict = merge(verdict, write_verdict(&target, root, base));
        }
    }
    for inner in substitutions(command) {
        verdict = merge(verdict, classify_in(inner, root, dir.clone()));
    }
    verdict
}

fn segment_verdict(
    prefix: &[&str],
    argv: &[&str],
    root: Option<&Path>,
    dir: &mut Option<PathBuf>,
) -> Verdict {
    if prefix.iter().any(|assignment| {
        GIT_REDIRECTS
            .iter()
            .any(|name| assignment.starts_with(*name))
    }) {
        return Verdict::Block("it redirects git's repository, index, or config".into());
    }
    let here = dir.clone();
    let here = here.as_deref();
    match argv.split_first() {
        Some((&("cd" | "pushd"), args)) => {
            *dir = change_dir(here, args);
            Verdict::Allow
        }
        Some((&"git", args)) => git_verdict(args),
        Some((&"rm", args)) => rm_verdict(args, root, here),
        Some((&("unlink" | "rmdir"), args)) => worst(&operands(args, &[]), |target| {
            target_verdict(target, false, root, here)
        }),
        Some((&"shred", args)) => worst(&operands(args, &[]), |target| {
            target_verdict(target, true, root, here)
        }),
        Some((&"truncate", args)) => worst(&operands(args, &["-s", "-r"]), |target| {
            write_verdict(target, root, here)
        }),
        Some((&"tee", args)) => worst(&operands(args, &[]), |target| {
            write_verdict(target, root, here)
        }),
        Some((&"cp", args)) => write_last(args, root, here),
        Some((&"mv", args)) => mv_verdict(args, root, here),
        Some((&("chmod" | "chown" | "chgrp"), args)) => recursive_verdict(args, root, here),
        Some((&"find", args)) => find_verdict(args, root, here),
        Some((name, args)) if INTERPRETERS.contains(name) => interpreter_verdict(args),
        Some((&("sh" | "bash" | "zsh"), args)) => shell_payload(args)
            .map_or(Verdict::Allow, |payload| {
                classify_in(payload, root, dir.clone())
            }),
        Some((&"eval", args)) => classify_in(&args.join(" "), root, dir.clone()),
        Some((&"dd", args)) if args.iter().any(|arg| arg.starts_with("of=/dev/")) => {
            Verdict::Block("dd writes to a device".into())
        }
        Some((name, _)) if name.starts_with("mkfs") => {
            Verdict::Block("mkfs formats a filesystem".into())
        }
        _ => Verdict::Allow,
    }
}

fn merge(current: Verdict, next: Verdict) -> Verdict {
    if matches!(current, Verdict::Block(_)) || matches!(next, Verdict::Block(_)) {
        return if matches!(current, Verdict::Block(_)) {
            current
        } else {
            next
        };
    }
    if matches!(current, Verdict::Warn(_)) {
        current
    } else {
        next
    }
}

fn worst(targets: &[&str], check: impl Fn(&str) -> Verdict) -> Verdict {
    targets.iter().fold(Verdict::Allow, |verdict, target| {
        merge(verdict, check(target))
    })
}

fn change_dir(current: Option<&Path>, args: &[&str]) -> Option<PathBuf> {
    let Some(&target) = args.first() else {
        return home_dir();
    };
    if target == "-" {
        return None;
    }
    resolve(target, current)
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
    if target.chars().any(|c| "$`*?[".contains(c)) {
        return None;
    }
    if target == "~" || target.starts_with("~/") {
        let rest = target.trim_start_matches('~').trim_start_matches('/');
        return home_dir().map(|home| home.join(rest));
    }
    if target.starts_with('~') {
        return None;
    }
    Some(PathBuf::from(target))
}

fn operands<'a>(args: &[&'a str], value_flags: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip_value = false;
    let mut options_open = true;
    for &arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if options_open && arg == "--" {
            options_open = false;
            continue;
        }
        if options_open && arg.starts_with('-') && arg.len() > 1 {
            skip_value = value_flags.contains(&arg);
            continue;
        }
        out.push(arg);
    }
    out
}

fn has_recursive_flag(args: &[&str]) -> bool {
    args.iter().take_while(|arg| **arg != "--").any(|arg| {
        *arg == "--recursive"
            || (arg.starts_with('-')
                && !arg.starts_with("--")
                && arg.chars().skip(1).any(|c| c == 'r' || c == 'R'))
    })
}

fn rm_verdict(args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    let recursive = has_recursive_flag(args);
    let targets = operands(args, &[]);
    if targets.is_empty() {
        return if recursive {
            Verdict::Block("recursive rm without explicit targets".into())
        } else {
            Verdict::Allow
        };
    }
    worst(&targets, |target| {
        target_verdict(target, recursive, root, dir)
    })
}

fn write_last(args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    match operands(args, &[]).last() {
        Some(destination) => write_verdict(destination, root, dir),
        None => Verdict::Allow,
    }
}

fn mv_verdict(args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    let targets = operands(args, &[]);
    let Some((destination, sources)) = targets.split_last() else {
        return Verdict::Allow;
    };
    let removed = worst(sources, |source| target_verdict(source, false, root, dir));
    merge(removed, write_verdict(destination, root, dir))
}

fn recursive_verdict(args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    if !has_recursive_flag(args) {
        return Verdict::Allow;
    }
    let targets = operands(args, &[]);
    worst(targets.get(1..).unwrap_or(&[]), |target| {
        target_verdict(target, true, root, dir)
    })
}

fn find_verdict(args: &[&str], root: Option<&Path>, dir: Option<&Path>) -> Verdict {
    let destructive = args.iter().enumerate().any(|(index, arg)| {
        *arg == "-delete"
            || (matches!(*arg, "-exec" | "-execdir" | "-ok")
                && args
                    .get(index + 1)
                    .is_some_and(|cmd| matches!(*cmd, "rm" | "unlink" | "shred" | "rmdir")))
    });
    if !destructive {
        return Verdict::Allow;
    }
    let starts: Vec<&str> = args
        .iter()
        .copied()
        .take_while(|arg| !arg.starts_with('-') && *arg != "(" && *arg != "!")
        .collect();
    let starts = if starts.is_empty() { vec!["."] } else { starts };
    worst(&starts, |start| {
        if resolve(start, dir).as_deref() == root {
            Verdict::Warn(format!("find -delete at the project root: {start}"))
        } else {
            target_verdict(start, true, root, dir)
        }
    })
}

fn interpreter_verdict(args: &[&str]) -> Verdict {
    let Some(position) = args
        .iter()
        .position(|arg| matches!(*arg, "-e" | "-c" | "--eval" | "-p"))
    else {
        return Verdict::Allow;
    };
    let Some(&code) = args.get(position + 1) else {
        return Verdict::Allow;
    };
    if !DESTRUCTIVE_CODE
        .iter()
        .any(|keyword| code.contains(*keyword))
    {
        return Verdict::Allow;
    }
    if ABSOLUTE_IN_CODE.iter().any(|marker| code.contains(*marker)) {
        Verdict::Block(
            "interpreter code deletes or spawns deletes at an absolute or home path".into(),
        )
    } else {
        Verdict::Warn("interpreter code deletes files".into())
    }
}

fn target_verdict(
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
    if path.components().any(|part| part.as_os_str() == ".git") {
        return Verdict::Block(format!("would delete git metadata: {target}"));
    }
    if is_protected(&path) {
        return Verdict::Block(format!("would delete a protected path: {target}"));
    }
    if path == Path::new("/") || home_dir().is_some_and(|home| normalize(&home) == path) {
        return Verdict::Block("target is the filesystem root or the home directory".into());
    }
    if in_system_root(&path) {
        return Verdict::Block(format!("would delete a system path: {target}"));
    }
    if root.is_some_and(|root| path != root && path.starts_with(root)) {
        return if recursive {
            Verdict::Warn(format!("recursive delete inside the project: {target}"))
        } else {
            Verdict::Allow
        };
    }
    if is_scratch(&path) {
        return if recursive {
            Verdict::Warn(format!("recursive delete in a scratch directory: {target}"))
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
    if is_protected(&path) {
        return Verdict::Block(format!("writes a protected path: {target}"));
    }
    if in_system_root(&path) {
        return Verdict::Block(format!("writes a system path: {target}"));
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

fn git_verdict(args: &[&str]) -> Verdict {
    let mut index = 0;
    while let Some(&token) = args.get(index) {
        let (config, step) = match token {
            "-c" | "--config-env" => (args.get(index + 1).copied(), 2),
            "-C" | "--namespace" => (None, 2),
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
        "stash"
            if rest
                .first()
                .is_some_and(|arg| matches!(*arg, "drop" | "clear")) =>
        {
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

fn is_force_push(arg: &str) -> bool {
    arg.starts_with("--force")
        || arg == "--mirror"
        || arg.starts_with('+')
        || (arg.starts_with('-') && !arg.starts_with("--") && arg.chars().skip(1).any(|c| c == 'f'))
}

fn is_forced_clean(arg: &str) -> bool {
    arg == "--force"
        || (arg.starts_with('-') && !arg.starts_with("--") && arg.chars().skip(1).any(|c| c == 'f'))
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
    match classify(&input.tool_input.command, root.as_deref()) {
        Verdict::Allow => {}
        Verdict::Warn(reason) => println!(
            "{}",
            json!({ "systemMessage": format!("agent-brain warning: {reason}") })
        ),
        Verdict::Block(reason) => println!(
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
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/work/repo";

    fn verdict(command: &str) -> Verdict {
        classify(command, Some(Path::new(ROOT)))
    }

    fn is_allowed(command: &str) -> bool {
        verdict(command) == Verdict::Allow
    }

    fn is_warned(command: &str) -> bool {
        matches!(verdict(command), Verdict::Warn(_))
    }

    fn is_blocked(command: &str) -> bool {
        matches!(verdict(command), Verdict::Block(_))
    }

    #[test]
    fn allows_routine_commands() {
        assert!(is_allowed("ls -la"));
        assert!(is_allowed("git status"));
        assert!(is_allowed("git push origin main"));
        assert!(is_allowed("rm build/out.txt"));
        assert!(is_allowed("rm -f notes.txt"));
        assert!(is_allowed("cd src && rm lib.rs"));
        assert!(is_allowed("echo rm -rf /"));
        assert!(is_allowed(
            "git commit -m \"remove -rf / and push --force\""
        ));
    }

    #[test]
    fn allows_ordinary_redirects_and_file_operations() {
        assert!(is_allowed("echo hello > out.txt"));
        assert!(is_allowed("echo x > /dev/null"));
        assert!(is_allowed("make 2>&1 | tee build/log.txt"));
        assert!(is_allowed("make 2>/dev/null"));
        assert!(is_allowed("git log --oneline > /tmp/log.txt"));
        assert!(is_allowed("grep -r 'a > b' src"));
        assert!(is_allowed("mv src/a.rs src/b.rs"));
        assert!(is_allowed("cp src/a.rs src/b.rs"));
        assert!(is_allowed("chmod 644 README.md"));
        assert!(is_allowed("truncate -s 0 build/out.log"));
    }

    #[test]
    fn allows_harmless_git_and_search_commands() {
        assert!(is_allowed("git config user.name dev"));
        assert!(is_allowed("git checkout -b feature"));
        assert!(is_allowed("git restore --staged src/lib.rs"));
        assert!(is_allowed("find . -name '*.rs' -print"));
        assert!(is_allowed("python3 -c 'print(1)'"));
    }

    #[test]
    fn warns_on_reversible_or_scoped_destruction() {
        assert!(is_warned("rm -rf build"));
        assert!(is_warned("rm -r -f build"));
        assert!(is_warned("rm -rf /tmp/probe-dir"));
        assert!(is_warned("git reset --hard HEAD~1"));
        assert!(is_warned("git clean -fd"));
        assert!(is_warned("git checkout ."));
        assert!(is_warned("git stash drop"));
        assert!(is_warned("find . -name '*.o' -delete"));
        assert!(is_warned("find . -exec rm -rf {} +"));
        assert!(is_warned(
            "node -e \"fs.rmSync('build', {recursive: true})\""
        ));
        assert!(is_warned("echo x > /Users/aeswibon/notes.txt"));
    }

    #[test]
    fn allows_non_recursive_scratch_removal() {
        assert!(is_allowed("rm /tmp/probe-file"));
    }

    #[test]
    fn blocks_force_pushes() {
        assert!(is_blocked("git push --force origin main"));
        assert!(is_blocked("git push -f"));
        assert!(is_blocked("git push -uf origin main"));
        assert!(is_blocked("git push origin +main"));
        assert!(is_blocked("git push --force-with-lease"));
        assert!(is_blocked("git push --mirror"));
    }

    #[test]
    fn blocks_recursive_deletes_outside_the_project() {
        assert!(is_blocked("rm -rf /"));
        assert!(is_blocked("rm -rf ~"));
        assert!(is_blocked("rm -rf $HOME"));
        assert!(is_blocked("rm -rf \"$PWD\"/.."));
        assert!(is_blocked("rm -rf ../.."));
        assert!(is_blocked("rm -rf /Users/aeswibon/Documents"));
        assert!(is_blocked("rm -rf /tmp"));
        assert!(is_blocked("rm -rf --no-preserve-root /"));
    }

    #[test]
    fn blocks_deletes_of_system_and_protected_paths() {
        assert!(is_blocked("rm /etc/hosts"));
        assert!(is_blocked("rm /tmp/probe-file/../../etc/hosts"));
        assert!(is_blocked("rm ~/.ssh/id_rsa"));
        assert!(is_blocked("unlink ~/.autonomic/memory/brain.db"));
        assert!(is_blocked("shred -u /etc/passwd"));
        assert!(is_blocked("chmod -R 777 /usr"));
    }

    #[test]
    fn blocks_writes_to_system_protected_and_device_paths() {
        assert!(is_blocked("echo x > /etc/hosts"));
        assert!(is_blocked("echo x >> /dev/sda"));
        assert!(is_blocked("echo x > ~/.zshrc"));
        assert!(is_blocked("echo x > \"/etc/hosts\""));
        assert!(is_blocked("tee /etc/passwd"));
        assert!(is_blocked("echo go | tee -a /etc/hosts"));
        assert!(is_blocked("cp payload.sh /usr/local/bin/tool"));
        assert!(is_blocked("mv src/a.rs /etc/a.rs"));
        assert!(is_blocked("truncate -s 0 /etc/hosts"));
    }

    #[test]
    fn blocks_git_alias_hook_path_and_repository_redirects() {
        assert!(is_blocked("git config alias.ci '!git commit -n'"));
        assert!(is_blocked("git config core.hooksPath /dev/null"));
        assert!(is_blocked("git -c alias.ci=commit commit -m x"));
        assert!(is_blocked("GIT_DIR=/tmp/other git commit -m x"));
        assert!(is_blocked("GIT_INDEX_FILE=/tmp/i git add ."));
        assert!(is_blocked("git --git-dir=/tmp/x status"));
        assert!(is_blocked("git --work-tree /tmp/x status"));
    }

    #[test]
    fn blocks_find_and_interpreter_deletes_at_absolute_paths() {
        assert!(is_blocked("find / -name x -delete"));
        assert!(is_blocked("find ~ -name cache -delete"));
        assert!(is_blocked("perl -e 'unlink \"/etc/hosts\"'"));
        assert!(is_blocked(
            "python3 -c \"import shutil; shutil.rmtree('/')\""
        ));
    }

    #[test]
    fn blocks_deletes_of_git_metadata() {
        assert!(is_blocked("rm -rf .git"));
        assert!(is_blocked("rm -f ./.git/index"));
    }

    #[test]
    fn blocks_directory_changes_that_escape_the_project() {
        assert!(is_blocked("cd / && rm -rf etc"));
        assert!(is_blocked("cd /etc; rm -rf hosts"));
        assert!(is_blocked("cd $SOMEWHERE && rm -rf build"));
        assert!(is_warned("cd src && rm -rf generated"));
    }

    #[test]
    fn blocks_unresolvable_recursive_targets() {
        assert!(is_blocked("rm -rf *"));
        assert!(is_blocked("rm -rf $DIR"));
        assert!(is_blocked("xargs rm -rf"));
        assert!(is_blocked("rm -rf"));
    }

    #[test]
    fn blocks_bypass_hidden_in_wrappers_and_substitutions() {
        assert!(is_blocked("sh -c \"rm -rf /\""));
        assert!(is_blocked("env -u X rm -rf /"));
        assert!(is_blocked("echo go | xargs -0 rm -rf /"));
        assert!(is_blocked("eval \"rm -rf ~\""));
        assert!(is_blocked("echo \"$(rm -rf /)\""));
        assert!(is_blocked("echo `rm -rf /`"));
        assert!(is_blocked("true; git push -f"));
    }

    #[test]
    fn blocks_device_writes_and_hook_bypass() {
        assert!(is_blocked("mkfs.ext4 /dev/disk2"));
        assert!(is_blocked("dd if=/dev/zero of=/dev/disk2"));
        assert!(is_blocked("git commit -n -m x"));
    }

    #[test]
    fn falls_back_to_block_without_a_known_project_root() {
        assert!(matches!(classify("rm -rf build", None), Verdict::Block(_)));
    }
}
