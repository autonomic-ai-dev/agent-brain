use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::db::store::BrainStore;
use crate::feature_map;
use crate::verify::{self, VerifySource};

const BLOCK_REASON: &str = "Verification is missing. Files changed in this repo have no passing `agent-brain verify -- <cmd>` run recorded after the last edit. Run the project's checks (for example `cargo test` or the verify suite), let them pass, then finish.";
const EVIDENCE_LIMIT: usize = 100;
const STOP_BLOCK_LIMIT: u32 = 3;
const STATE_FILE: &str = "stop-gate-state.json";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StopInput {
    pub cwd: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BlockState {
    session_id: String,
    edit_ms: i64,
    blocks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub exit_code: i32,
    pub source: Option<String>,
    pub code_hash: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block(&'static str),
}

fn verified(code_hash: &str, evidence: &[Evidence]) -> bool {
    evidence.iter().any(|e| {
        e.exit_code == 0
            && matches!(e.source.as_deref(), Some("cli" | "orchestrator"))
            && e.code_hash.as_deref() == Some(code_hash)
    })
}

pub fn decide(verified: bool, blocks_so_far: u32) -> Decision {
    if verified || blocks_so_far >= STOP_BLOCK_LIMIT {
        Decision::Allow
    } else {
        Decision::Block(BLOCK_REASON)
    }
}

fn prior_blocks(prior: &BlockState, session_id: &str, edit_ms: i64) -> u32 {
    if prior.session_id == session_id && prior.edit_ms == edit_ms {
        prior.blocks
    } else {
        0
    }
}

fn load_state(path: &Path) -> BlockState {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &BlockState) -> Result<()> {
    fs::write(path, serde_json::to_string(state)?)?;
    Ok(())
}

fn git_root(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8(out.stdout).ok()?;
    Some(PathBuf::from(root.trim_end()))
}

fn changed_paths(root: &Path) -> Vec<String> {
    let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .filter_map(|path| path.rsplit(" -> ").next())
        .map(str::to_string)
        .collect()
}

fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn last_edit_ms(root: &Path) -> Option<i64> {
    changed_paths(root)
        .iter()
        .filter_map(|path| root.join(path).metadata().ok()?.modified().ok())
        .filter_map(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64)
        .max()
}

fn markdown_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".md") && name != "README.md")
        .collect()
}

fn feature_gap(root: &Path) -> Option<String> {
    let features_dir = root.join(".verify/features");
    if !features_dir.is_dir() {
        return None;
    }
    feature_map::gap(
        &changed_paths(root),
        &feature_map::load(&features_dir),
        &markdown_names(&root.join(".verify/knowledge-base")),
    )
}

fn load_evidence(store: &BrainStore, root: &Path) -> Result<Vec<Evidence>> {
    let has_harness = root.join(".verify").is_dir();
    Ok(store
        .list_verify_outcomes(EVIDENCE_LIMIT)?
        .into_iter()
        .filter(|row| !has_harness || verify::repo_artifact_path(root, &row.id).is_file())
        .map(|row| Evidence {
            exit_code: row.exit_code,
            source: row.source,
            code_hash: row.code_hash,
        })
        .collect())
}

pub fn run_stop_gate() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: StopInput = serde_json::from_str(&raw).unwrap_or_default();
    let cwd = input
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let config = Config::load()?;
    let store = BrainStore::open(&config.db_path)?;
    let state_file = config.db_path.with_file_name(STATE_FILE);
    let session_id = input.session_id.unwrap_or_default();
    let mut reasons = Vec::new();
    if let Some(root) = git_root(&cwd) {
        let repo_state =
            state_file.with_file_name(format!("stop-gate-state-{}.json", repo_name(&root)));
        if let Some(reason) = evaluate_stop(&store, &root, &session_id, &repo_state)? {
            reasons.push(reason);
        }
    }
    if !reasons.is_empty() {
        let reason = reasons.join("\n\n");
        let headline: String = reason
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect();
        if let Err(err) = crate::gate_events::record(
            &config.logs_dir.join("gate-events.jsonl"),
            "stop-block",
            &headline,
        ) {
            tracing::warn!(error = %err, "gate event write failed");
        }
        println!(
            "{}",
            serde_json::json!({ "decision": "block", "reason": reason })
        );
    }
    Ok(())
}

fn evaluate_stop(
    store: &BrainStore,
    root: &Path,
    session_id: &str,
    state_file: &Path,
) -> Result<Option<String>> {
    let Some(edit) = last_edit_ms(root) else {
        return Ok(None);
    };
    let gap = feature_gap(root);
    let code_changed = changed_paths(root)
        .iter()
        .any(|path| verify::is_code_path(path));
    let evidence = load_evidence(store, root)?;
    let passed = code_changed
        && verify::code_tree_hash(root)
            .as_deref()
            .is_some_and(|hash| verified(hash, &evidence));
    if (!code_changed || passed) && gap.is_none() {
        return Ok(None);
    }
    let blocks = prior_blocks(&load_state(state_file), session_id, edit);
    let step = if code_changed && !passed {
        verify::run_default_step(store, root, VerifySource::Cli)?
    } else {
        None
    };
    let reason = match (&step, &gap) {
        (Some(run), _) if !run.report.success => failure_reason(run),
        (_, Some(gap)) => gap.clone(),
        (Some(_), None) => return Ok(None),
        (None, None) => BLOCK_REASON.to_string(),
    };
    let verdict = match &gap {
        Some(_) if blocks < STOP_BLOCK_LIMIT => Decision::Block(BLOCK_REASON),
        Some(_) => Decision::Allow,
        None => decide(passed, blocks),
    };
    match verdict {
        Decision::Block(_) => {
            let next = BlockState {
                session_id: session_id.to_string(),
                edit_ms: edit,
                blocks: blocks + 1,
            };
            if let Err(err) = save_state(state_file, &next) {
                eprintln!("stop-gate: could not persist block count: {err:#}");
            }
            Ok(Some(reason))
        }
        Decision::Allow => {
            eprintln!(
                "stop-gate: stopping after {blocks} blocks; verification or feature maps are still incomplete for the last edit"
            );
            Ok(None)
        }
    }
}

fn failure_reason(run: &verify::StepRun) -> String {
    format!(
        "Verification failed: `{}` exited with {}. Fix the failures below, then finish again.\n\n{}",
        run.report.cmd,
        run.report.exit_code,
        run.output.trim_end()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn evidence(exit_code: i32, source: Option<&str>, code_hash: Option<&str>) -> Evidence {
        Evidence {
            exit_code,
            source: source.map(str::to_string),
            code_hash: code_hash.map(str::to_string),
        }
    }

    #[test]
    fn blocks_code_change_without_a_run_for_its_tree() {
        assert!(matches!(decide(false, 0), Decision::Block(_)));
    }

    #[test]
    fn blocks_run_recorded_for_another_tree() {
        let ev = [evidence(0, Some("cli"), Some("old-tree"))];
        assert!(!verified("new-tree", &ev));
    }

    #[test]
    fn blocks_failing_run_for_the_same_tree() {
        let ev = [evidence(1, Some("cli"), Some("tree"))];
        assert!(!verified("tree", &ev));
    }

    #[test]
    fn blocks_run_not_from_trusted_source() {
        let ev = [evidence(0, Some("mcp"), Some("tree"))];
        assert!(!verified("tree", &ev));
    }

    #[test]
    fn allows_passing_cli_run_for_the_same_tree() {
        let ev = [evidence(0, Some("cli"), Some("tree"))];
        assert!(verified("tree", &ev));
        assert_eq!(decide(true, 0), Decision::Allow);
    }

    #[test]
    fn allows_passing_orchestrator_run_for_the_same_tree() {
        let ev = [evidence(0, Some("orchestrator"), Some("tree"))];
        assert!(verified("tree", &ev));
    }

    #[test]
    fn keeps_blocking_below_the_block_limit() {
        assert!(matches!(
            decide(false, STOP_BLOCK_LIMIT - 1),
            Decision::Block(_)
        ));
    }

    #[test]
    fn allows_stop_once_block_limit_is_reached_without_verification() {
        assert_eq!(decide(false, STOP_BLOCK_LIMIT), Decision::Allow);
    }

    #[test]
    fn only_code_paths_need_a_verification_run() {
        assert!(verify::is_code_path("agent-brain/src/engine.rs"));
        assert!(verify::is_code_path("agent-brain/hooks/route_gate.py"));
        assert!(verify::is_code_path("agent-brain/Cargo.toml"));
        assert!(!verify::is_code_path("agent-brain/.verify/features/x.md"));
        assert!(!verify::is_code_path("agent-brain/README.md"));
    }

    #[test]
    fn counts_blocks_only_for_the_same_session_and_edit() {
        let prior = BlockState {
            session_id: "s1".into(),
            edit_ms: 2_000,
            blocks: 2,
        };
        assert_eq!(prior_blocks(&prior, "s1", 2_000), 2);
        assert_eq!(prior_blocks(&prior, "s2", 2_000), 0);
        assert_eq!(prior_blocks(&prior, "s1", 2_500), 0);
    }

    #[test]
    fn reports_edit_time_for_untracked_file() {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        std::fs::write(dir.path().join("note.txt"), "edit").unwrap();
        assert!(last_edit_ms(dir.path()).is_some_and(|ms| ms > 0));
    }

    #[test]
    fn reports_no_edit_for_clean_repo() {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert_eq!(last_edit_ms(dir.path()), None);
    }

    fn edited_repo(dir: &Path, verify_toml: Option<&str>) {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "-q"])
            .status()
            .unwrap();
        if let Some(body) = verify_toml {
            fs::create_dir_all(dir.join(".verify")).unwrap();
            fs::write(dir.join(".verify/verify.toml"), body).unwrap();
        }
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/changed.rs"), b"fn changed() {}").unwrap();
    }

    fn doc_edited_repo(dir: &Path, verify_toml: &str) {
        edited_repo(dir, Some(verify_toml));
        fs::remove_file(dir.join("src/changed.rs")).unwrap();
        fs::write(dir.join("changed.txt"), b"edit").unwrap();
    }

    fn gate_store(dir: &Path) -> BrainStore {
        BrainStore::open(&dir.join("t.db")).unwrap()
    }

    #[test]
    fn passing_default_step_lets_the_stop_through_and_is_recorded() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(
            dir.path(),
            Some("[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n"),
        );
        let state = aux.path().join("state.json");
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            None
        );
        assert_eq!(store.list_verify_outcomes(10).unwrap().len(), 1);
    }

    #[test]
    fn failing_default_step_blocks_with_its_command_and_output() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(
            dir.path(),
            Some("[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"echo broken; exit 3\"]\n"),
        );
        let state = aux.path().join("state.json");
        let reason = evaluate_stop(&store, dir.path(), "s1", &state)
            .unwrap()
            .expect("failing step must block");
        assert!(reason.contains("exit 3"));
        assert!(reason.contains("broken"));
    }

    #[test]
    fn stop_is_allowed_after_the_retry_limit_while_the_step_still_fails() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(
            dir.path(),
            Some("[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"exit 1\"]\n"),
        );
        let state = aux.path().join("state.json");
        for _ in 0..STOP_BLOCK_LIMIT {
            assert!(evaluate_stop(&store, dir.path(), "s1", &state)
                .unwrap()
                .is_some());
        }
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            None
        );
    }

    #[test]
    fn edits_without_a_default_step_still_block() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(dir.path(), None);
        let state = aux.path().join("state.json");
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            Some(BLOCK_REASON.to_string())
        );
    }

    #[test]
    fn doc_only_edit_runs_no_verify_step() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        doc_edited_repo(
            dir.path(),
            "[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"exit 1\"]\n",
        );
        let state = aux.path().join("state.json");
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            None
        );
        assert!(store.list_verify_outcomes(10).unwrap().is_empty());
    }

    #[test]
    fn code_tree_hash_moves_on_code_edits_and_not_on_doc_edits() {
        let dir = TempDir::new().unwrap();
        edited_repo(dir.path(), None);
        let before = verify::code_tree_hash(dir.path()).unwrap();
        fs::write(dir.path().join("changed.txt"), b"doc edit").unwrap();
        assert_eq!(verify::code_tree_hash(dir.path()).unwrap(), before);
        fs::write(dir.path().join("src/changed.rs"), b"fn edited() {}").unwrap();
        assert_ne!(verify::code_tree_hash(dir.path()).unwrap(), before);
    }

    #[test]
    fn code_edit_after_a_passing_run_needs_another_run() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(
            dir.path(),
            Some("[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n"),
        );
        let state = aux.path().join("state.json");
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            None
        );
        fs::write(dir.path().join("src/changed.rs"), b"fn changed_again() {}").unwrap();
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            None
        );
        assert_eq!(store.list_verify_outcomes(10).unwrap().len(), 2);
    }

    #[test]
    fn unclaimed_code_blocks_even_after_verification_passes() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(
            dir.path(),
            Some("[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n"),
        );
        fs::create_dir_all(dir.path().join(".verify/features")).unwrap();
        let state = aux.path().join("state.json");
        let reason = evaluate_stop(&store, dir.path(), "s1", &state)
            .unwrap()
            .expect("unclaimed code must block");
        assert!(reason.contains("`src/changed.rs`"));
    }

    #[test]
    fn verified_stop_passes_once_the_owning_feature_is_updated() {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        edited_repo(
            dir.path(),
            Some("[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n"),
        );
        fs::create_dir_all(dir.path().join(".verify/features")).unwrap();
        fs::write(
            dir.path().join(".verify/features/gate.md"),
            b"# gate\n\n## Code\n\n- `src/changed.rs`\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(".verify/knowledge-base")).unwrap();
        fs::write(dir.path().join(".verify/knowledge-base/gate.md"), b"# gate").unwrap();
        let state = aux.path().join("state.json");
        assert_eq!(
            evaluate_stop(&store, dir.path(), "s1", &state).unwrap(),
            None
        );
    }

    const MATRIX_VERIFY: &str = "[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"true\"]\n";
    const MATRIX_FAILING_VERIFY: &str = "[verify]\ncommands = [\"sh -c\"]\n\n[verify.default]\ncommand = [\"sh\", \"-c\", \"exit 1\"]\n";
    const GATE_MAP: &str = ".verify/features/gate.md";
    const GATE_KB: &str = ".verify/knowledge-base/gate.md";

    struct MatrixRepo {
        dir: TempDir,
        aux: TempDir,
        store: BrainStore,
    }

    fn write_file(root: &Path, path: &str, text: &str) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn committed_repo(verify_toml: &str, baseline: Vec<(&str, String)>) -> MatrixRepo {
        let dir = TempDir::new().unwrap();
        let aux = TempDir::new().unwrap();
        let store = gate_store(aux.path());
        git_in(dir.path(), &["init", "-q"]);
        write_file(dir.path(), ".verify/verify.toml", verify_toml);
        for (path, text) in &baseline {
            write_file(dir.path(), path, text);
        }
        git_in(dir.path(), &["add", "-A"]);
        git_in(
            dir.path(),
            &[
                "-c",
                "user.name=gate",
                "-c",
                "user.email=gate@test",
                "commit",
                "-qm",
                "baseline",
            ],
        );
        MatrixRepo { dir, aux, store }
    }

    fn evaluate(repo: &MatrixRepo) -> Option<String> {
        let state = repo.aux.path().join("state.json");
        evaluate_stop(&repo.store, repo.dir.path(), "s1", &state).unwrap()
    }

    fn stop_after(repo: &MatrixRepo, changes: Vec<(&str, String)>) -> Option<String> {
        for (path, text) in &changes {
            write_file(repo.dir.path(), path, text);
        }
        evaluate(repo)
    }

    fn feature_map(claims: &[&str]) -> String {
        let mut text = String::from("# feature\n\n## Code\n\n");
        for claim in claims {
            text.push_str(&format!("- `{claim}`\n"));
        }
        text.push_str("\n## How to get to it\n");
        text
    }

    fn gate_baseline() -> Vec<(&'static str, String)> {
        vec![
            (GATE_MAP, feature_map(&["src/a.rs"])),
            (GATE_KB, "# gate".to_string()),
        ]
    }

    #[test]
    fn matrix_no_change_allows_the_stop() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        assert_eq!(stop_after(&repo, vec![]), None);
    }

    #[test]
    fn matrix_docs_only_change_allows_the_stop() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        assert_eq!(
            stop_after(&repo, vec![("README.md", "docs".to_string())]),
            None
        );
    }

    #[test]
    fn matrix_unclaimed_code_is_named_in_the_block() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let reason = stop_after(&repo, vec![("src/b.rs", "fn b() {}".to_string())]).unwrap();
        assert!(reason.contains("`src/b.rs`"));
    }

    #[test]
    fn matrix_owned_code_without_its_map_update_is_blocked() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let reason = stop_after(&repo, vec![("src/a.rs", "fn a() {}".to_string())]).unwrap();
        assert!(reason.contains("`gate.md`"));
    }

    #[test]
    fn matrix_owned_code_with_map_but_no_entry_is_blocked() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let reason = stop_after(
            &repo,
            vec![
                ("src/a.rs", "fn a() {}".to_string()),
                (GATE_MAP, feature_map(&["src/a.rs", "src/c.rs"])),
            ],
        )
        .unwrap();
        assert!(reason.contains("`gate.md`"));
    }

    #[test]
    fn matrix_owned_code_with_map_and_entry_passes() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let changes = vec![
            ("src/a.rs", "fn a() {}".to_string()),
            (GATE_MAP, feature_map(&["src/a.rs", "src/c.rs"])),
            (GATE_KB, "# gate, updated".to_string()),
        ];
        assert_eq!(stop_after(&repo, changes), None);
    }

    #[test]
    fn matrix_shared_file_needs_every_owner_updated() {
        let repo = committed_repo(
            MATRIX_VERIFY,
            vec![
                (".verify/features/one.md", feature_map(&["src/shared.rs"])),
                (".verify/features/two.md", feature_map(&["src"])),
                (".verify/knowledge-base/one.md", "# one".to_string()),
                (".verify/knowledge-base/two.md", "# two".to_string()),
            ],
        );
        let reason = stop_after(
            &repo,
            vec![
                ("src/shared.rs", "fn s() {}".to_string()),
                (
                    ".verify/features/one.md",
                    feature_map(&["src/shared.rs"]) + "\n",
                ),
                (
                    ".verify/knowledge-base/one.md",
                    "# one, updated".to_string(),
                ),
            ],
        )
        .unwrap();
        assert!(reason.contains("`two.md`"));
        assert!(!reason.contains("`one.md`"));
    }

    #[test]
    fn matrix_shared_file_with_every_owner_updated_passes() {
        let repo = committed_repo(
            MATRIX_VERIFY,
            vec![
                (".verify/features/one.md", feature_map(&["src/shared.rs"])),
                (".verify/features/two.md", feature_map(&["src"])),
                (".verify/knowledge-base/one.md", "# one".to_string()),
                (".verify/knowledge-base/two.md", "# two".to_string()),
            ],
        );
        let changes = vec![
            ("src/shared.rs", "fn s() {}".to_string()),
            (
                ".verify/features/one.md",
                feature_map(&["src/shared.rs"]) + "\n",
            ),
            (
                ".verify/knowledge-base/one.md",
                "# one, updated".to_string(),
            ),
            (".verify/features/two.md", feature_map(&["src"]) + "\n"),
            (
                ".verify/knowledge-base/two.md",
                "# two, updated".to_string(),
            ),
        ];
        assert_eq!(stop_after(&repo, changes), None);
    }

    #[test]
    fn matrix_new_feature_for_new_code_passes_when_complete() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let changes = vec![
            ("src/new.rs", "fn n() {}".to_string()),
            (".verify/features/new.md", feature_map(&["src/new.rs"])),
            (".verify/knowledge-base/new.md", "# new".to_string()),
        ];
        assert_eq!(stop_after(&repo, changes), None);
    }

    #[test]
    fn matrix_new_feature_without_its_entry_is_blocked() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let reason = stop_after(
            &repo,
            vec![
                ("src/new.rs", "fn n() {}".to_string()),
                (".verify/features/new.md", feature_map(&["src/new.rs"])),
            ],
        )
        .unwrap();
        assert!(reason.contains("`new.md`"));
    }

    #[test]
    fn matrix_feature_without_an_entry_on_disk_is_named() {
        let repo = committed_repo(
            MATRIX_VERIFY,
            vec![
                (GATE_MAP, feature_map(&["src/a.rs"])),
                (GATE_KB, "# gate".to_string()),
                (".verify/features/other.md", feature_map(&["src/other.rs"])),
            ],
        );
        let changes = vec![
            ("src/a.rs", "fn a() {}".to_string()),
            (GATE_MAP, feature_map(&["src/a.rs"]) + "\n"),
            (GATE_KB, "# gate, updated".to_string()),
        ];
        let reason = stop_after(&repo, changes).unwrap();
        assert!(reason.contains("`other.md`"));
        assert!(reason.contains("no knowledge-base entry"));
    }

    #[test]
    fn matrix_new_untracked_directory_names_each_file() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let reason = stop_after(&repo, vec![("src/newmod/x.rs", "fn x() {}".to_string())]).unwrap();
        assert!(reason.contains("`src/newmod/x.rs`"));
    }

    #[test]
    fn matrix_a_claim_is_a_path_prefix_not_a_string_prefix() {
        let repo = committed_repo(
            MATRIX_VERIFY,
            vec![
                (GATE_MAP, feature_map(&["src"])),
                (GATE_KB, "# gate".to_string()),
            ],
        );
        let reason = stop_after(&repo, vec![("srcx/file.rs", "fn f() {}".to_string())]).unwrap();
        assert!(reason.contains("`srcx/file.rs`"));
    }

    #[test]
    fn matrix_paths_outside_the_code_section_claim_nothing() {
        let repo = committed_repo(
            MATRIX_VERIFY,
            vec![
                (
                    GATE_MAP,
                    "# gate\n\n## Gotchas\n\n- `src/a.rs`\n".to_string(),
                ),
                (GATE_KB, "# gate".to_string()),
            ],
        );
        let reason = stop_after(&repo, vec![("src/a.rs", "fn a() {}".to_string())]).unwrap();
        assert!(reason.contains("`src/a.rs`"));
        assert!(reason.contains("outside every feature's"));
    }

    #[test]
    fn matrix_failing_verification_blocks_even_when_owned_work_is_complete() {
        let repo = committed_repo(MATRIX_FAILING_VERIFY, gate_baseline());
        let changes = vec![
            ("src/a.rs", "fn a() {}".to_string()),
            (GATE_MAP, feature_map(&["src/a.rs"]) + "\n"),
            (GATE_KB, "# gate, updated".to_string()),
        ];
        let reason = stop_after(&repo, changes).unwrap();
        assert!(reason.contains("exited with 1"));
    }

    #[test]
    fn matrix_stop_is_allowed_after_the_block_limit_while_code_stays_unclaimed() {
        let repo = committed_repo(MATRIX_VERIFY, gate_baseline());
        let first = stop_after(&repo, vec![("src/b.rs", "fn b() {}".to_string())]);
        assert!(first.is_some());
        for _ in 1..STOP_BLOCK_LIMIT {
            assert!(evaluate(&repo).is_some());
        }
        assert_eq!(evaluate(&repo), None);
    }

    #[test]
    fn hook_outside_a_repo_checks_no_child_repos() {
        let root = TempDir::new().unwrap();
        let changed = root.path().join("changed");
        fs::create_dir_all(&changed).unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&changed)
            .args(["init", "-q"])
            .status()
            .unwrap();
        fs::write(changed.join("a.rs"), "fn a() {}").unwrap();
        assert!(git_root(root.path()).is_none());
        assert_eq!(
            git_root(&changed).map(|root| repo_name(&root)),
            Some("changed".to_string())
        );
    }
}
