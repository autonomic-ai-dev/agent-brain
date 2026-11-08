use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TEAM_RULES: &str = "# Team rules\n\nNo force pushes to main.\n";

fn install(home: &Path, project: &Path, host: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-brain"))
        .args(["install", host])
        .current_dir(project)
        .env("HOME", home)
        .env("AGENT_BRAIN_HOME", home.join("ab"))
        .output()
        .expect("run agent-brain install")
}

fn repo() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project");
    let path = project.path();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "mode@example.invalid"],
        vec!["config", "user.name", "Mode Test"],
        vec!["commit", "-q", "--allow-empty", "-m", "initial"],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(path)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?}");
    }
    project
}

fn check_host(host: &str, file: &str) {
    let home = tempfile::tempdir().expect("home");
    let project = repo();
    let path: PathBuf = project.path().join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, TEAM_RULES).unwrap();

    let first = install(home.path(), project.path(), host);
    assert!(
        first.status.success(),
        "install --{host} failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let after_first = fs::read_to_string(&path).unwrap();
    assert!(
        after_first.contains("<!-- agent-brain:begin mode -->"),
        "{host}: mode block missing from {file}"
    );
    assert!(
        after_first.contains("agstack mode"),
        "{host}: agstack mode text missing from {file}"
    );
    assert!(
        after_first.contains("No force pushes to main."),
        "{host}: team rules were lost from {file}"
    );

    let second = install(home.path(), project.path(), host);
    assert!(second.status.success());
    let after_second = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after_first, after_second,
        "{host}: a second install must not change {file}"
    );
    assert_eq!(
        after_second
            .matches("<!-- agent-brain:begin mode -->")
            .count(),
        1,
        "{host}: the block must not be duplicated"
    );
}

#[test]
fn codex_project_install_adds_mode_to_agents_md() {
    check_host("--codex", "AGENTS.md");
}

#[test]
fn opencode_project_install_adds_mode_to_agents_md() {
    check_host("--opencode", "AGENTS.md");
}

#[test]
fn antigravity_project_install_adds_mode_to_agents_md() {
    check_host("--antigravity", "AGENTS.md");
}

#[test]
fn gemini_project_install_adds_mode_to_gemini_md() {
    check_host("--gemini", "GEMINI.md");
}

#[test]
fn vscode_project_install_keeps_team_copilot_instructions() {
    check_host("--vscode", ".github/copilot-instructions.md");
    let home = tempfile::tempdir().expect("home");
    let project = repo();
    let path = project.path().join(".github/copilot-instructions.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, TEAM_RULES).unwrap();
    assert!(install(home.path(), project.path(), "--vscode")
        .status
        .success());
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("<!-- agent-brain:begin copilot -->"));
    assert!(text.contains("No force pushes to main."));
}
