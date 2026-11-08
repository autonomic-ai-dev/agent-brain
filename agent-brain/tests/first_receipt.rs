use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn agent_brain(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-brain"))
        .args(args)
        .current_dir(cwd)
        .env("AGENT_BRAIN_HOME", home)
        .output()
        .expect("run agent-brain")
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", message]);
}

fn new_project() -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("home");
    let root = tempfile::tempdir().expect("project");
    let path = root.path();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "receipts@example.invalid"]);
    git(path, &["config", "user.name", "Receipts Test"]);
    git(path, &["config", "gc.auto", "0"]);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(path.join("Makefile"), "test:\n\ttest -f .pass\n").unwrap();
    commit_all(path, "initial");
    (home, root)
}

#[test]
fn first_receipt_follows_the_documented_path() {
    let (home, project) = new_project();
    let home = home.path();
    let root = project.path();

    let init = agent_brain(
        home,
        root,
        &["verify", "init", "--dir", root.to_str().unwrap()],
    );
    assert!(
        init.status.success(),
        "verify init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let before = agent_brain(home, root, &["verify", "receipt"]);
    assert_eq!(before.status.code(), Some(1));
    assert!(stdout(&before).contains("Verify receipt: NOT VERIFIED"));
    assert!(stdout(&before).contains("agent-brain verify -- <cmd>"));

    let failing = agent_brain(home, root, &["verify", "--", "make", "test"]);
    assert!(
        !failing.status.success(),
        "make test should fail before .pass exists"
    );
    let failed_receipt = agent_brain(home, root, &["verify", "receipt"]);
    assert_eq!(failed_receipt.status.code(), Some(1));
    assert!(stdout(&failed_receipt).contains("Verify receipt: FAILED"));
    assert!(stdout(&failed_receipt).contains("exit code: 2"));

    fs::write(root.join(".pass"), "").unwrap();
    let passing = agent_brain(home, root, &["verify", "--", "make", "test"]);
    assert!(
        passing.status.success(),
        "make test should pass: {}",
        String::from_utf8_lossy(&passing.stderr)
    );
    let receipt = agent_brain(home, root, &["verify", "receipt"]);
    assert_eq!(receipt.status.code(), Some(0));
    let text = stdout(&receipt);
    assert!(text.contains("Verify receipt: PASSED"), "{text}");
    assert!(text.contains("command:   make test"), "{text}");
    assert!(text.contains("exit code: 0"), "{text}");
    assert!(text.contains("source:    cli"), "{text}");
}

#[test]
fn editing_code_after_a_pass_makes_the_receipt_stale_until_rerun() {
    let (home, project) = new_project();
    let home = home.path();
    let root = project.path();
    agent_brain(
        home,
        root,
        &["verify", "init", "--dir", root.to_str().unwrap()],
    );
    fs::write(root.join(".pass"), "").unwrap();
    assert!(agent_brain(home, root, &["verify", "--", "make", "test"])
        .status
        .success());
    assert_eq!(
        agent_brain(home, root, &["verify", "receipt"])
            .status
            .code(),
        Some(0)
    );

    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    let stale = agent_brain(home, root, &["verify", "receipt"]);
    assert_eq!(stale.status.code(), Some(1));
    assert!(stdout(&stale).contains("Verify receipt: NOT VERIFIED"));

    assert!(agent_brain(home, root, &["verify", "--", "make", "test"])
        .status
        .success());
    assert_eq!(
        agent_brain(home, root, &["verify", "receipt"])
            .status
            .code(),
        Some(0)
    );
}

#[test]
fn a_new_commit_needs_its_own_verify_run() {
    let (home, project) = new_project();
    let home = home.path();
    let root = project.path();
    agent_brain(
        home,
        root,
        &["verify", "init", "--dir", root.to_str().unwrap()],
    );
    fs::write(root.join(".pass"), "").unwrap();
    assert!(agent_brain(home, root, &["verify", "--", "make", "test"])
        .status
        .success());
    assert_eq!(
        agent_brain(home, root, &["verify", "receipt"])
            .status
            .code(),
        Some(0)
    );

    fs::write(root.join("src/lib.rs"), "pub fn added() {}\n").unwrap();
    commit_all(root, "add lib");
    let after_commit = agent_brain(home, root, &["verify", "receipt"]);
    assert_eq!(
        after_commit.status.code(),
        Some(1),
        "old run must not cover the new commit"
    );
    assert!(stdout(&after_commit).contains("Verify receipt: NOT VERIFIED"));
}

#[test]
fn markdown_and_json_receipts_carry_the_same_verdict() {
    let (home, project) = new_project();
    let home = home.path();
    let root = project.path();
    agent_brain(
        home,
        root,
        &["verify", "init", "--dir", root.to_str().unwrap()],
    );
    fs::write(root.join(".pass"), "").unwrap();
    assert!(agent_brain(home, root, &["verify", "--", "make", "test"])
        .status
        .success());

    let markdown = agent_brain(home, root, &["verify", "receipt", "--markdown"]);
    assert!(
        stdout(&markdown).starts_with("**Verify receipt: PASSED**"),
        "{}",
        stdout(&markdown)
    );
    assert!(stdout(&markdown).contains("| Command | `make test` |"));

    let json = agent_brain(home, root, &["verify", "receipt", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&json)).expect("json receipt");
    assert_eq!(value["verdict"], "passed");
    assert_eq!(value["evidence"]["exit_code"], 0);
    assert_eq!(value["evidence"]["cmd"], "make test");
}
