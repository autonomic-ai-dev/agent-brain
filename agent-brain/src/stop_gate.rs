use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::{find_repo_root, Config};
use crate::db::store::BrainStore;
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
    pub recorded_at: i64,
    pub source: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block(&'static str),
}

fn has_verification(edit: i64, evidence: &[Evidence]) -> bool {
    evidence.iter().any(|e| {
        e.exit_code == 0
            && e.recorded_at >= edit
            && matches!(e.source.as_deref(), Some("cli" | "orchestrator"))
    })
}

pub fn decide(last_edit_ms: Option<i64>, evidence: &[Evidence], blocks_so_far: u32) -> Decision {
    let Some(edit) = last_edit_ms else {
        return Decision::Allow;
    };
    if has_verification(edit, evidence) || blocks_so_far >= STOP_BLOCK_LIMIT {
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

fn last_edit_ms(cwd: &Path) -> Option<i64> {
    let root = git_root(cwd)?;
    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .filter_map(|path| path.rsplit(" -> ").next())
        .filter_map(|path| root.join(path).metadata().ok()?.modified().ok())
        .filter_map(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64)
        .max()
}

fn load_evidence(store: &BrainStore) -> Result<Vec<Evidence>> {
    Ok(store
        .list_verify_outcomes(EVIDENCE_LIMIT)?
        .into_iter()
        .map(|row| Evidence {
            exit_code: row.exit_code,
            recorded_at: row.recorded_at,
            source: row.source,
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
    if let Some(reason) = evaluate_stop(&store, &cwd, &session_id, &state_file)? {
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
    cwd: &Path,
    session_id: &str,
    state_file: &Path,
) -> Result<Option<String>> {
    let Some(edit) = last_edit_ms(cwd) else {
        return Ok(None);
    };
    let evidence = load_evidence(store)?;
    if has_verification(edit, &evidence) {
        return Ok(None);
    }
    let blocks = prior_blocks(&load_state(state_file), session_id, edit);
    let root = find_repo_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let step = verify::run_default_step(store, &root, VerifySource::Cli)?;
    if let Some(run) = &step {
        if run.report.success {
            return Ok(None);
        }
    }
    let reason = match &step {
        Some(run) => failure_reason(run),
        None => BLOCK_REASON.to_string(),
    };
    match decide(Some(edit), &evidence, blocks) {
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
                "stop-gate: stopping without verification after {blocks} blocks; no passing verify run follows the last edit"
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

    fn evidence(exit_code: i32, recorded_at: i64, source: Option<&str>) -> Evidence {
        Evidence {
            exit_code,
            recorded_at,
            source: source.map(str::to_string),
        }
    }

    #[test]
    fn allows_stop_when_no_repo_edits() {
        assert_eq!(decide(None, &[], 0), Decision::Allow);
    }

    #[test]
    fn blocks_edits_without_verification() {
        assert!(matches!(decide(Some(2_000), &[], 0), Decision::Block(_)));
    }

    #[test]
    fn blocks_verification_recorded_before_last_edit() {
        let ev = [evidence(0, 1_000, Some("cli"))];
        assert!(matches!(decide(Some(2_000), &ev, 0), Decision::Block(_)));
    }

    #[test]
    fn blocks_failing_verification_after_edit() {
        let ev = [evidence(1, 3_000, Some("cli"))];
        assert!(matches!(decide(Some(2_000), &ev, 0), Decision::Block(_)));
    }

    #[test]
    fn blocks_verification_not_from_trusted_source() {
        let ev = [evidence(0, 3_000, Some("mcp"))];
        assert!(matches!(decide(Some(2_000), &ev, 0), Decision::Block(_)));
    }

    #[test]
    fn allows_passing_cli_verification_after_last_edit() {
        let ev = [evidence(0, 3_000, Some("cli"))];
        assert_eq!(decide(Some(2_000), &ev, 0), Decision::Allow);
    }

    #[test]
    fn allows_orchestrator_verification_after_last_edit() {
        let ev = [evidence(0, 3_000, Some("orchestrator"))];
        assert_eq!(decide(Some(2_000), &ev, 0), Decision::Allow);
    }

    #[test]
    fn keeps_blocking_below_the_block_limit() {
        assert!(matches!(
            decide(Some(2_000), &[], STOP_BLOCK_LIMIT - 1),
            Decision::Block(_)
        ));
    }

    #[test]
    fn allows_stop_once_block_limit_is_reached_without_verification() {
        assert_eq!(decide(Some(2_000), &[], STOP_BLOCK_LIMIT), Decision::Allow);
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
}
