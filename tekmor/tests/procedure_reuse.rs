use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tekmor::db::store::BrainStore;
use tekmor::retrieval::recall_procedures;

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

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("project");
    let path = root.path();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "procedure@example.invalid"]);
    git(path, &["config", "user.name", "Procedure Test"]);
    git(path, &["config", "gc.auto", "0"]);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(path.join("Makefile"), "test:\n\ttest -f .pass\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "initial"]);
    root
}

#[test]
fn a_procedure_learned_in_one_task_is_recalled_for_a_second_task() {
    let home = tempfile::tempdir().expect("home");
    let root = project();
    let path = root.path();

    let init = tekmor(
        home.path(),
        path,
        &["verify", "init", "--dir", path.to_str().unwrap()],
    );
    assert!(init.status.success(), "verify init failed");

    let first_attempt = tekmor(home.path(), path, &["verify", "--", "make", "test"]);
    assert!(
        !first_attempt.status.success(),
        "task one starts with a failing test"
    );

    fs::write(path.join(".pass"), "").unwrap();
    let fixed = tekmor(home.path(), path, &["verify", "--", "make", "test"]);
    assert!(fixed.status.success(), "task one ends with a passing test");

    let db = home.path().join("data/brain.db");
    assert!(
        db.is_file(),
        "verify runs must write the brain database at {}",
        db.display()
    );
    let store = BrainStore::open(&db).expect("open brain database");

    let recalled =
        recall_procedures(&store, "make test", Some("verification"), 5).expect("recall procedures");
    let procedure = recalled
        .iter()
        .find(|item| item.node_id == "make")
        .expect("the make test procedure from task one is recalled for task two");

    assert_eq!(
        procedure.total_runs, 2,
        "both attempts from task one are counted"
    );
    assert!(
        (procedure.success_rate - 0.5).abs() < f64::EPSILON,
        "one of two attempts passed"
    );
    assert_eq!(procedure.task_kind.as_deref(), Some("verification"));
    assert!(procedure.score > 0.0);
}

#[test]
fn an_unrelated_task_does_not_recall_the_procedure() {
    let home = tempfile::tempdir().expect("home");
    let root = project();
    let path = root.path();
    tekmor(
        home.path(),
        path,
        &["verify", "init", "--dir", path.to_str().unwrap()],
    );
    fs::write(path.join(".pass"), "").unwrap();
    assert!(
        tekmor(home.path(), path, &["verify", "--", "make", "test"])
            .status
            .success()
    );

    let store = BrainStore::open(&home.path().join("data/brain.db")).expect("open brain database");
    let recalled = recall_procedures(&store, "rotate the production database password", None, 5)
        .expect("recall procedures");
    let make_match = recalled.iter().find(|item| item.node_id == "make");
    assert!(
        make_match.is_none_or(|item| item.score < 0.3),
        "an unrelated query must not rank the make procedure highly: {make_match:?}"
    );
}
