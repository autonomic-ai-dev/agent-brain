use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use serde_json::Value;
use tempfile::TempDir;

fn isolated_home() -> &'static Path {
    static HOME: OnceLock<TempDir> = OnceLock::new();
    HOME.get_or_init(|| tempfile::tempdir().expect("temp tekmor home"))
        .path()
}

fn run_bash_gate(command: &str, cwd: &str) -> Value {
    run_bash_gate_in(isolated_home(), command, cwd)
}

fn run_bash_gate_in(home: &Path, command: &str, cwd: &str) -> Value {
    let payload = serde_json::json!({
        "cwd": cwd,
        "tool_input": { "command": command }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_tekmor"))
        .arg("bash-gate")
        .env("TEKMOR_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tekmor bash-gate");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.to_string().as_bytes())
        .expect("write hook payload");
    let output = child.wait_with_output().expect("wait for bash-gate");
    assert!(output.status.success(), "bash-gate exited with failure");
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    if stdout.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(stdout.trim()).expect("hook output is json")
}

fn decision(output: &Value) -> Option<&str> {
    output["hookSpecificOutput"]["permissionDecision"].as_str()
}

fn warning(output: &Value) -> Option<&str> {
    output["systemMessage"].as_str()
}

const PROJECT: &str = "/work/repo";

#[test]
fn allowed_commands_produce_no_hook_output() {
    let commands = [
        "git status",
        "ls -la",
        "rm build/out.txt",
        "git commit -m \"remove -n flag and --no-verify text\"",
        "echo rm -rf /",
    ];
    for command in commands {
        assert_eq!(run_bash_gate(command, PROJECT), Value::Null, "{command}");
    }
}

#[test]
fn reversible_destruction_warns_without_denying() {
    let commands = [
        "rm -rf build",
        "git reset --hard HEAD~1",
        "rm -rf /tmp/probe-dir",
    ];
    for command in commands {
        let output = run_bash_gate(command, PROJECT);
        assert!(decision(&output).is_none(), "{command} must not deny");
        assert!(
            warning(&output).is_some_and(|text| text.starts_with("tekmor warning")),
            "{command} must warn"
        );
    }
}

#[test]
fn destructive_and_bypass_commands_are_denied() {
    let commands = [
        "git push --force origin main",
        "git push origin +main",
        "rm -rf /",
        "rm -rf ~",
        "rm -rf $HOME",
        "rm -rf ../..",
        "rm -rf .git",
        "cd / && rm -rf etc",
        "rm -rf *",
        "sh -c \"rm -rf /\"",
        "echo \"$(rm -rf /)\"",
        "git commit -n -m x",
        "env -u X git commit --no-verify -m x",
        "mkfs.ext4 /dev/disk2",
        "echo x > /etc/hosts",
        "echo x > ~/.zshrc",
        "tee /etc/passwd",
        "git config alias.ci '!git commit -n'",
        "GIT_DIR=/tmp/other git commit -m x",
        "find / -name x -delete",
        "perl -e 'unlink \"/etc/hosts\"'",
        "shred -u /etc/passwd",
    ];
    for command in commands {
        let output = run_bash_gate(command, PROJECT);
        assert_eq!(decision(&output), Some("deny"), "{command} must be denied");
        assert!(
            output["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .is_some_and(|reason| reason.starts_with("tekmor blocked this command")),
            "{command} must explain the block"
        );
    }
}

#[test]
fn gate_events_are_written_under_the_configured_home() {
    let home = tempfile::tempdir().expect("temp tekmor home");
    let output = run_bash_gate_in(home.path(), "rm -rf /", PROJECT);
    assert_eq!(decision(&output), Some("deny"));

    let log = std::fs::read_to_string(home.path().join("logs/gate-events.jsonl"))
        .expect("gate log under the configured home");
    let events: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).expect("event is json"))
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "bash-block");
}
