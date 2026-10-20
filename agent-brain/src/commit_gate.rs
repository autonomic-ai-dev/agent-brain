use std::io::Read;

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

const DENY_REASON: &str = "Commits must not skip git hooks. The coding-policy comment guard runs in the pre-commit hook. Remove --no-verify, -n, and any core.hooksPath override, then commit again.";

const WRAPPERS: &[&str] = &["command", "exec", "env", "nohup", "time", "xargs"];

const COMMANDS: &[&str] = &["git", "rm", "sh", "bash", "zsh", "eval", "dd"];

const VALUE_FLAGS: &[&str] = &[
    "-m",
    "-F",
    "-C",
    "-c",
    "--message",
    "--file",
    "--author",
    "--date",
    "--template",
    "--cleanup",
    "--fixup",
    "--squash",
    "--reuse-message",
    "--reedit-message",
];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ToolUse {
    pub(crate) tool_input: ToolInput,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ToolInput {
    pub(crate) command: String,
}

pub fn bypasses_git_hooks(command: &str) -> bool {
    split_segments(command).iter().any(|segment| {
        let words: Vec<&str> = segment.iter().map(String::as_str).collect();
        segment_bypasses(&words)
    }) || substitutions(command).into_iter().any(bypasses_git_hooks)
}

fn segment_bypasses(words: &[&str]) -> bool {
    let start = command_start(words);
    if words[..start].iter().any(|word| is_hooks_env(word)) {
        return true;
    }
    match words[start..].split_first() {
        Some((&"git", args)) => git_bypasses(args),
        Some((&"eval", args)) => bypasses_git_hooks(&args.join(" ")),
        Some((&("sh" | "bash" | "zsh"), args)) => {
            shell_payload(args).is_some_and(bypasses_git_hooks)
        }
        _ => false,
    }
}

pub(crate) fn command_start(words: &[&str]) -> usize {
    let index = words.iter().take_while(|word| is_assignment(word)).count();
    if words.get(index).is_some_and(|word| WRAPPERS.contains(word)) {
        return words[index..]
            .iter()
            .position(|word| COMMANDS.contains(word) || word.starts_with("mkfs"))
            .map_or(words.len(), |offset| index + offset);
    }
    index
}

fn git_bypasses(args: &[&str]) -> bool {
    let mut index = 0;
    while let Some(&token) = args.get(index) {
        match token {
            "-c" | "--config-env" => {
                if args.get(index + 1).is_some_and(|kv| is_hooks_path(kv)) {
                    return true;
                }
                index += 2;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" => index += 2,
            _ if token.starts_with("--config-env=") => {
                if is_hooks_path(&token["--config-env=".len()..]) {
                    return true;
                }
                index += 1;
            }
            _ if token.starts_with("-c") => {
                if is_hooks_path(&token[2..]) {
                    return true;
                }
                index += 1;
            }
            _ if token.starts_with('-') => index += 1,
            _ => break,
        }
    }
    args.get(index) == Some(&"commit") && commit_skips_hooks(&args[index + 1..])
}

fn commit_skips_hooks(args: &[&str]) -> bool {
    let mut index = 0;
    while let Some(&arg) = args.get(index) {
        if arg == "--" {
            return false;
        }
        if VALUE_FLAGS.contains(&arg) {
            index += 2;
            continue;
        }
        if skips_hooks_flag(arg) {
            return true;
        }
        index += 1;
    }
    false
}

fn skips_hooks_flag(arg: &str) -> bool {
    if let Some(long) = arg.strip_prefix("--no-") {
        return !long.is_empty() && "verify".starts_with(long);
    }
    match arg.strip_prefix('-') {
        Some(cluster) if !cluster.starts_with('-') => cluster
            .chars()
            .take_while(|c| !"mFCc".contains(*c))
            .any(|c| c == 'n'),
        _ => false,
    }
}

fn is_hooks_path(config: &str) -> bool {
    config.to_ascii_lowercase().starts_with("core.hookspath=")
}

fn is_hooks_env(assignment: &str) -> bool {
    assignment.starts_with("GIT_CONFIG")
        && assignment.to_ascii_lowercase().contains("core.hookspath")
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

pub(crate) fn shell_payload<'a>(args: &[&'a str]) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == "-c")?;
    args.get(position + 1).copied()
}

pub(crate) fn substitutions(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            if let Some(len) = command[index + 1..].find('`') {
                found.push(&command[index + 1..index + 1 + len]);
                index += len + 2;
                continue;
            }
        } else if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'(') {
            let start = index + 2;
            let mut depth = 1;
            let end = bytes[start..]
                .iter()
                .enumerate()
                .find_map(|(offset, byte)| {
                    match byte {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    (depth == 0).then_some(start + offset)
                });
            if let Some(end) = end {
                found.push(&command[start..end]);
                index = end + 1;
                continue;
            }
        }
        index += 1;
    }
    found
}

pub(crate) fn split_segments(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(ch);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    in_word = true;
                }
            }
            (None, c) if c.is_whitespace() && c != '\n' => {
                push_word(&mut current, &mut word, &mut in_word);
            }
            (None, '\n' | ';' | '|' | '&' | '(' | ')') => {
                push_word(&mut current, &mut word, &mut in_word);
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    push_word(&mut current, &mut word, &mut in_word);
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn push_word(tokens: &mut Vec<String>, word: &mut String, in_word: &mut bool) {
    if *in_word {
        tokens.push(std::mem::take(word));
        *in_word = false;
    }
}

pub fn run_commit_gate() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: ToolUse = serde_json::from_str(&raw).unwrap_or_default();
    if bypasses_git_hooks(&input.tool_input.command) {
        println!(
            "{}",
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": DENY_REASON
                }
            })
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_ordinary_commits() {
        assert!(!bypasses_git_hooks("git commit -m \"fix: parser\""));
        assert!(!bypasses_git_hooks("git status"));
        assert!(!bypasses_git_hooks("git -c user.name=x commit -m msg"));
        assert!(!bypasses_git_hooks("git -C repo commit -am 'fix'"));
    }

    #[test]
    fn allows_hook_flag_text_inside_quoted_message() {
        assert!(!bypasses_git_hooks("git commit -m \"remove -n flag\""));
        assert!(!bypasses_git_hooks(
            "git commit -m 'use --no-verify carefully'"
        ));
        assert!(!bypasses_git_hooks("git commit -m -n"));
        assert!(!bypasses_git_hooks("git commit -mn"));
    }

    #[test]
    fn allows_non_commit_git_commands_with_similar_flags() {
        assert!(!bypasses_git_hooks("git log -n 5"));
        assert!(!bypasses_git_hooks("echo commit -n"));
        assert!(!bypasses_git_hooks("echo git commit -n"));
        assert!(!bypasses_git_hooks("git commit -m \"$(cat msg.txt)\""));
    }

    #[test]
    fn denies_no_verify_and_short_form() {
        assert!(bypasses_git_hooks("git commit --no-verify -m x"));
        assert!(bypasses_git_hooks("git commit -n -m x"));
        assert!(bypasses_git_hooks("git commit -nm x"));
        assert!(bypasses_git_hooks("git commit --no-v -m x"));
        assert!(bypasses_git_hooks("git commit -m ok --no-verify"));
    }

    #[test]
    fn denies_hooks_path_override_in_any_case_and_position() {
        assert!(bypasses_git_hooks(
            "git -c core.hooksPath=/dev/null commit -m x"
        ));
        assert!(bypasses_git_hooks(
            "git -c CORE.HOOKSPATH=/dev/null commit -m x"
        ));
        assert!(bypasses_git_hooks(
            "git -C repo -c core.hooksPath=/dev/null commit -m x"
        ));
        assert!(bypasses_git_hooks(
            "git --config-env=core.hooksPath=HOOKS commit -m x"
        ));
        assert!(bypasses_git_hooks(
            "GIT_CONFIG_PARAMETERS=\"'core.hooksPath=/dev/null'\" git commit -m x"
        ));
    }

    #[test]
    fn denies_bypass_hidden_in_chained_commands() {
        assert!(bypasses_git_hooks(
            "git commit -m ok && git commit --no-verify -m x"
        ));
        assert!(bypasses_git_hooks("cd repo; git commit -n -m x"));
        assert!(bypasses_git_hooks("make build || git commit -n"));
    }

    #[test]
    fn denies_bypass_hidden_in_wrappers() {
        assert!(bypasses_git_hooks("sh -c \"git commit -n -m x\""));
        assert!(bypasses_git_hooks("bash -c 'git commit --no-verify'"));
        assert!(bypasses_git_hooks("eval \"git commit -n\""));
        assert!(bypasses_git_hooks("env FOO=1 git commit -n"));
        assert!(bypasses_git_hooks("env -u FOO git commit -n"));
        assert!(bypasses_git_hooks("command git commit --no-verify"));
        assert!(bypasses_git_hooks("echo go | xargs -0 git commit -n"));
    }

    #[test]
    fn denies_bypass_hidden_in_substitutions() {
        assert!(bypasses_git_hooks("echo \"$(git commit -n -m x)\""));
        assert!(bypasses_git_hooks("echo `git commit -n`"));
        assert!(bypasses_git_hooks("git commit -m \"$(git commit -n)\""));
    }
}
