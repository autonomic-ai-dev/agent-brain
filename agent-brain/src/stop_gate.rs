use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use serde::Deserialize;

use crate::config::Config;
use crate::db::store::BrainStore;

const BLOCK_REASON: &str = "Verification is missing. Files changed in this repo have no passing `agent-brain verify -- <cmd>` run recorded after the last edit. Run the project's checks (for example `cargo test` or the verify suite), let them pass, then finish.";
const EVIDENCE_LIMIT: usize = 100;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StopInput {
    pub cwd: Option<String>,
    pub stop_hook_active: bool,
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

pub fn decide(
    last_edit_ms: Option<i64>,
    evidence: &[Evidence],
    stop_hook_active: bool,
) -> Decision {
    let Some(edit) = last_edit_ms else {
        return Decision::Allow;
    };
    let verified = evidence.iter().any(|e| {
        e.exit_code == 0
            && e.recorded_at >= edit
            && matches!(e.source.as_deref(), Some("cli" | "orchestrator"))
    });
    if verified || stop_hook_active {
        Decision::Allow
    } else {
        Decision::Block(BLOCK_REASON)
    }
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
    let edit = last_edit_ms(&cwd);
    if edit.is_none() {
        return Ok(());
    }
    let config = Config::load()?;
    let store = BrainStore::open(&config.db_path)?;
    let evidence = load_evidence(&store)?;
    if let Decision::Block(reason) = decide(edit, &evidence, input.stop_hook_active) {
        println!(
            "{}",
            serde_json::json!({ "decision": "block", "reason": reason })
        );
    }
    Ok(())
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
        assert_eq!(decide(None, &[], false), Decision::Allow);
    }

    #[test]
    fn blocks_edits_without_verification() {
        assert!(matches!(
            decide(Some(2_000), &[], false),
            Decision::Block(_)
        ));
    }

    #[test]
    fn blocks_verification_recorded_before_last_edit() {
        let ev = [evidence(0, 1_000, Some("cli"))];
        assert!(matches!(
            decide(Some(2_000), &ev, false),
            Decision::Block(_)
        ));
    }

    #[test]
    fn blocks_failing_verification_after_edit() {
        let ev = [evidence(1, 3_000, Some("cli"))];
        assert!(matches!(
            decide(Some(2_000), &ev, false),
            Decision::Block(_)
        ));
    }

    #[test]
    fn blocks_verification_not_from_trusted_source() {
        let ev = [evidence(0, 3_000, Some("mcp"))];
        assert!(matches!(
            decide(Some(2_000), &ev, false),
            Decision::Block(_)
        ));
    }

    #[test]
    fn allows_passing_cli_verification_after_last_edit() {
        let ev = [evidence(0, 3_000, Some("cli"))];
        assert_eq!(decide(Some(2_000), &ev, false), Decision::Allow);
    }

    #[test]
    fn allows_orchestrator_verification_after_last_edit() {
        let ev = [evidence(0, 3_000, Some("orchestrator"))];
        assert_eq!(decide(Some(2_000), &ev, false), Decision::Allow);
    }

    #[test]
    fn allows_second_stop_attempt_to_avoid_loop() {
        assert_eq!(decide(Some(2_000), &[], true), Decision::Allow);
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
}
