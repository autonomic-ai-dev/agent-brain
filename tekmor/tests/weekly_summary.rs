use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn tekmor(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tekmor"))
        .args(args)
        .current_dir(cwd)
        .env("TEKMOR_HOME", home)
        .output()
        .expect("run tekmor")
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

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("project");
    let path = root.path();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "summary@example.invalid"]);
    git(path, &["config", "user.name", "Summary Test"]);
    git(path, &["config", "gc.auto", "0"]);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(path.join("Makefile"), "test:\n\ttest -f .pass\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "initial"]);
    root
}

#[test]
fn empty_history_still_renders_a_measured_summary() {
    let home = tempfile::tempdir().expect("home");
    let root = project();
    let output = tekmor(
        home.path(),
        root.path(),
        &["stats", "--markdown", "--days", "7"],
    );
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(
        text.starts_with("**tekmor summary, last 7 days (measured)**"),
        "{text}"
    );
    assert!(
        text.contains("| Verify runs passed / failed | 0 / 0 |"),
        "{text}"
    );
    assert!(text.contains("| Verify pass rate | no runs |"), "{text}");
    assert!(
        !text.contains("upper bound"),
        "estimates must not appear: {text}"
    );
}

#[test]
fn summary_counts_a_real_verify_run() {
    let home = tempfile::tempdir().expect("home");
    let root = project();
    let path = root.path();
    assert!(tekmor(
        home.path(),
        path,
        &["verify", "init", "--dir", path.to_str().unwrap()]
    )
    .status
    .success());
    fs::write(path.join(".pass"), "").unwrap();
    assert!(
        tekmor(home.path(), path, &["verify", "--", "make", "test"])
            .status
            .success()
    );

    let text = stdout(&tekmor(home.path(), path, &["stats", "--markdown"]));
    assert!(
        text.contains("| Verify runs passed / failed | 1 / 0 |"),
        "{text}"
    );
    assert!(text.contains("| Verify pass rate | 100.0% |"), "{text}");
}

#[test]
fn json_output_is_unchanged_by_the_markdown_flag() {
    let home = tempfile::tempdir().expect("home");
    let root = project();
    let json = stdout(&tekmor(home.path(), root.path(), &["stats", "--json"]));
    let value: serde_json::Value = serde_json::from_str(&json).expect("json stats");
    assert_eq!(value["period_days"], 7);
}
