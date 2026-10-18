use std::io::Read;

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

const DENY_REASON: &str = "Commits must not skip git hooks. The coding-policy comment guard runs in the pre-commit hook. Remove --no-verify, -n, and any core.hooksPath override, then commit again.";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolUse {
    tool_input: ToolInput,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolInput {
    command: String,
}

pub fn bypasses_git_hooks(command: &str) -> bool {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if !(tokens.contains(&"git") && tokens.contains(&"commit")) {
        return false;
    }
    tokens
        .iter()
        .any(|token| matches!(*token, "--no-verify" | "-n") || token.starts_with("core.hooksPath"))
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
    }

    #[test]
    fn denies_no_verify_and_short_form() {
        assert!(bypasses_git_hooks("git commit --no-verify -m x"));
        assert!(bypasses_git_hooks("git commit -n -m x"));
    }

    #[test]
    fn denies_hooks_path_override() {
        assert!(bypasses_git_hooks(
            "git -c core.hooksPath=/dev/null commit -m x"
        ));
    }

    #[test]
    fn ignores_non_commit_commands_with_similar_flags() {
        assert!(!bypasses_git_hooks("git log -n 5"));
        assert!(!bypasses_git_hooks("echo commit -n"));
    }
}
