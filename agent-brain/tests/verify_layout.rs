use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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

fn initialised_project() -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("home");
    let root = tempfile::tempdir().expect("project");
    let path = root.path();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "layout@example.invalid"]);
    git(path, &["config", "user.name", "Layout Test"]);
    git(path, &["config", "gc.auto", "0"]);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(path.join("Makefile"), "test:\n\ttest -f .pass\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "initial"]);
    let init = agent_brain(
        home.path(),
        path,
        &["verify", "init", "--dir", path.to_str().unwrap()],
    );
    assert!(init.status.success(), "verify init failed");
    (home, root)
}

fn only_run_dir(root: &Path) -> PathBuf {
    let runs: Vec<PathBuf> = fs::read_dir(root.join(".verify/artifacts"))
        .expect("artifacts dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(
        runs.len(),
        1,
        "expected exactly one run folder, found {runs:?}"
    );
    runs.into_iter().next().unwrap()
}

fn summary(root: &Path) -> String {
    fs::read_to_string(only_run_dir(root).join("summary.txt")).expect("summary.txt")
}

#[test]
fn init_writes_the_agstack_layout() {
    let (_home, project) = initialised_project();
    let root = project.path();
    let verify = root.join(".verify");
    assert!(verify.join("verify.toml").is_file());
    assert!(verify.join("README.md").is_file());
    assert!(verify.join("artifacts/.gitkeep").is_file());
    assert!(verify.join("functional/.gitkeep").is_file());
    let mode = fs::metadata(verify.join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "run.sh must be executable");
    let readme = fs::read_to_string(verify.join("README.md")).unwrap();
    assert!(readme.contains("agstack verification skill"));
    assert!(readme.contains("functional/"));
}

#[test]
fn failing_unit_phase_fails_the_run_and_keeps_its_log() {
    let (home, project) = initialised_project();
    let root = project.path();
    let output = agent_brain(home.path(), root, &["verify", "--", ".verify/run.sh"]);
    assert!(!output.status.success(), "run must fail without .pass");
    let text = summary(root);
    assert!(text.contains("[fail] unit"), "{text}");
    assert!(
        text.contains("[skip] doctor: no .verify/doctor.sh"),
        "{text}"
    );
    assert!(
        text.contains("[skip] static: no .verify/static.sh"),
        "{text}"
    );
    assert!(
        text.contains("[skip] functional: no .verify/functional/run.sh"),
        "{text}"
    );
    assert!(only_run_dir(root).join("unit.log").is_file());
}

#[test]
fn passing_run_with_functional_suite_passes_every_run_phase() {
    let (home, project) = initialised_project();
    let root = project.path();
    fs::write(root.join(".pass"), "").unwrap();
    fs::write(
        root.join(".verify/functional/run.sh"),
        "#!/usr/bin/env bash\necho ok\n",
    )
    .unwrap();
    let output = agent_brain(home.path(), root, &["verify", "--", ".verify/run.sh"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = summary(root);
    assert!(text.contains("[pass] unit"), "{text}");
    assert!(text.contains("[pass] functional"), "{text}");
    let receipt = agent_brain(home.path(), root, &["verify", "receipt"]);
    assert_eq!(receipt.status.code(), Some(0));
    assert!(stdout(&receipt).contains("command:   .verify/run.sh"));
}

#[test]
fn failing_doctor_stops_the_run_before_any_test() {
    let (home, project) = initialised_project();
    let root = project.path();
    fs::write(root.join(".pass"), "").unwrap();
    fs::write(
        root.join(".verify/doctor.sh"),
        "#!/usr/bin/env bash\nexit 1\n",
    )
    .unwrap();
    let output = agent_brain(home.path(), root, &["verify", "--", ".verify/run.sh"]);
    assert!(!output.status.success());
    let text = summary(root);
    assert!(text.contains("[fail] doctor"), "{text}");
    assert!(text.contains("doctor failed; no tests were run"), "{text}");
    assert!(
        !text.contains("unit"),
        "unit must not run after a failed doctor: {text}"
    );
}

#[test]
fn feature_argument_is_recorded_in_the_run_summary() {
    let (home, project) = initialised_project();
    let root = project.path();
    fs::write(root.join(".pass"), "").unwrap();
    let output = agent_brain(
        home.path(),
        root,
        &["verify", "--", ".verify/run.sh", "receipts"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(summary(root).contains("feature=receipts"));
}
