use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::db::store::{BrainStore, VerifyOutcomeRow};
use crate::project_key;
use crate::verify;

const RECEIPT_SCAN_LIMIT: usize = 200;
const SHORT_SHA_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    Failed,
    NotVerified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub id: String,
    pub cmd: String,
    pub exit_code: i32,
    pub recorded_at: i64,
    pub source: String,
    pub artifact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Receipt {
    pub verdict: Verdict,
    pub head_sha: Option<String>,
    pub code_hash: Option<String>,
    pub evidence: Option<Evidence>,
}

fn ran_under_verify_runner(source: Option<&str>) -> bool {
    matches!(source.unwrap_or("cli"), "cli" | "orchestrator")
}

pub fn build_receipt(
    rows: &[VerifyOutcomeRow],
    head_sha: Option<&str>,
    code_hash: Option<&str>,
) -> Receipt {
    let evidence = head_sha.zip(code_hash).and_then(|(head, hash)| {
        rows.iter()
            .find(|row| {
                row.head_sha.as_deref() == Some(head)
                    && row.code_hash.as_deref() == Some(hash)
                    && ran_under_verify_runner(row.source.as_deref())
            })
            .map(|row| Evidence {
                id: row.id.clone(),
                cmd: row.cmd.clone(),
                exit_code: row.exit_code,
                recorded_at: row.recorded_at,
                source: row.source.clone().unwrap_or_else(|| "cli".to_string()),
                artifact: None,
            })
    });
    let verdict = match &evidence {
        None => Verdict::NotVerified,
        Some(found) if found.exit_code == 0 => Verdict::Passed,
        Some(_) => Verdict::Failed,
    };
    Receipt {
        verdict,
        head_sha: head_sha.map(str::to_string),
        code_hash: code_hash.map(str::to_string),
        evidence,
    }
}

pub fn receipt_for_project(store: &BrainStore, root: &Path) -> Result<Receipt> {
    let head = verify::git_head_sha(root);
    let code = verify::code_tree_hash(root);
    let rows = match project_key::project_key_if_known(root) {
        Some(key) => store.list_verify_outcomes_for_project(&key, RECEIPT_SCAN_LIMIT)?,
        None => Vec::new(),
    };
    let mut receipt = build_receipt(&rows, head.as_deref(), code.as_deref());
    if let Some(evidence) = receipt.evidence.as_mut() {
        let artifact = verify::repo_artifact_path(root, &evidence.id);
        if artifact.is_file() {
            evidence.artifact = Some(
                artifact
                    .strip_prefix(root)
                    .unwrap_or(&artifact)
                    .display()
                    .to_string(),
            );
        }
    }
    Ok(receipt)
}

fn short(value: &str) -> &str {
    value.get(..SHORT_SHA_LEN).unwrap_or(value)
}

fn recorded_at_text(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| millis.to_string())
}

fn headline(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Passed => "PASSED",
        Verdict::Failed => "FAILED",
        Verdict::NotVerified => "NOT VERIFIED",
    }
}

fn not_verified_hint(receipt: &Receipt) -> &'static str {
    if receipt.head_sha.is_none() {
        "no git commit found for this directory; run inside a git repository with at least one commit"
    } else {
        "no verify run recorded for this commit and code state; run `agent-brain verify -- <cmd>`"
    }
}

pub fn render_text(receipt: &Receipt) -> String {
    let mut out = format!("Verify receipt: {}\n", headline(receipt.verdict));
    match &receipt.evidence {
        Some(evidence) => {
            out.push_str(&format!("  command:   {}\n", evidence.cmd));
            out.push_str(&format!("  exit code: {}\n", evidence.exit_code));
            out.push_str(&format!(
                "  recorded:  {}\n",
                recorded_at_text(evidence.recorded_at)
            ));
            out.push_str(&format!("  source:    {}\n", evidence.source));
            if let Some(artifact) = &evidence.artifact {
                out.push_str(&format!("  artifact:  {artifact}\n"));
            }
        }
        None => out.push_str(&format!("  {}\n", not_verified_hint(receipt))),
    }
    if let Some(head) = &receipt.head_sha {
        out.push_str(&format!("  commit:    {}\n", short(head)));
    }
    if let Some(hash) = &receipt.code_hash {
        out.push_str(&format!("  code hash: {}\n", short(hash)));
    }
    out
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('`', "'")
}

pub fn render_markdown(receipt: &Receipt) -> String {
    let mut out = format!("**Verify receipt: {}**\n\n", headline(receipt.verdict));
    match &receipt.evidence {
        Some(evidence) => {
            out.push_str("| Field | Value |\n|---|---|\n");
            out.push_str(&format!(
                "| Command | `{}` |\n",
                markdown_cell(&evidence.cmd)
            ));
            out.push_str(&format!("| Exit code | {} |\n", evidence.exit_code));
            out.push_str(&format!(
                "| Recorded | {} |\n",
                recorded_at_text(evidence.recorded_at)
            ));
            out.push_str(&format!("| Source | {} |\n", evidence.source));
            if let Some(artifact) = &evidence.artifact {
                out.push_str(&format!(
                    "| Proof artifact | `{}` |\n",
                    markdown_cell(artifact)
                ));
            }
            if let Some(head) = &receipt.head_sha {
                out.push_str(&format!("| Commit | `{}` |\n", short(head)));
            }
            if let Some(hash) = &receipt.code_hash {
                out.push_str(&format!("| Code hash | `{}` |\n", short(hash)));
            }
        }
        None => out.push_str(&format!("{}.\n", not_verified_hint(receipt))),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";
    const HASH: &str = "9f8e7d6c5b4a3921fedcba9876543210ffeeddcc";

    fn row(
        cmd: &str,
        exit: i32,
        head: Option<&str>,
        hash: Option<&str>,
        source: Option<&str>,
    ) -> VerifyOutcomeRow {
        VerifyOutcomeRow {
            id: format!("id-{cmd}-{exit}"),
            route_log_id: None,
            exit_code: exit,
            cause_tag: None,
            verify_config_hash: None,
            scenario_dir_hash: None,
            cmd: cmd.to_string(),
            device_id: None,
            recorded_at: 1_794_000_000_000,
            source: source.map(str::to_string),
            code_hash: hash.map(str::to_string),
            head_sha: head.map(str::to_string),
        }
    }

    #[test]
    fn passing_run_for_this_commit_and_code_state_is_passed() {
        let rows = [row("cargo test", 0, Some(HEAD), Some(HASH), Some("cli"))];
        let receipt = build_receipt(&rows, Some(HEAD), Some(HASH));
        assert_eq!(receipt.verdict, Verdict::Passed);
        assert_eq!(
            receipt.evidence.as_ref().map(|e| e.cmd.as_str()),
            Some("cargo test")
        );
    }

    #[test]
    fn failing_run_for_this_commit_is_failed_with_its_exit_code() {
        let rows = [row("cargo test", 101, Some(HEAD), Some(HASH), Some("cli"))];
        let receipt = build_receipt(&rows, Some(HEAD), Some(HASH));
        assert_eq!(receipt.verdict, Verdict::Failed);
        assert_eq!(receipt.evidence.map(|e| e.exit_code), Some(101));
    }

    #[test]
    fn newest_matching_run_decides_even_when_an_older_one_passed() {
        let rows = [
            row("cargo test", 1, Some(HEAD), Some(HASH), Some("cli")),
            row("cargo test", 0, Some(HEAD), Some(HASH), Some("cli")),
        ];
        assert_eq!(
            build_receipt(&rows, Some(HEAD), Some(HASH)).verdict,
            Verdict::Failed
        );
    }

    #[test]
    fn run_for_another_commit_does_not_count() {
        let other = "ffffffffffffffffffffffffffffffffffffffff";
        let rows = [row("cargo test", 0, Some(other), Some(HASH), Some("cli"))];
        let receipt = build_receipt(&rows, Some(HEAD), Some(HASH));
        assert_eq!(receipt.verdict, Verdict::NotVerified);
        assert!(receipt.evidence.is_none());
    }

    #[test]
    fn run_for_an_earlier_code_state_on_the_same_commit_does_not_count() {
        let rows = [row("cargo test", 0, Some(HEAD), Some("0000"), Some("cli"))];
        assert_eq!(
            build_receipt(&rows, Some(HEAD), Some(HASH)).verdict,
            Verdict::NotVerified
        );
    }

    #[test]
    fn rows_recorded_before_commit_tracking_never_count() {
        let rows = [row("cargo test", 0, None, Some(HASH), Some("cli"))];
        assert_eq!(
            build_receipt(&rows, Some(HEAD), Some(HASH)).verdict,
            Verdict::NotVerified
        );
    }

    #[test]
    fn hook_and_mcp_records_do_not_count_as_a_verify_run() {
        let rows = [
            row("cargo test", 0, Some(HEAD), Some(HASH), Some("hook")),
            row("cargo test", 0, Some(HEAD), Some(HASH), Some("mcp")),
        ];
        assert_eq!(
            build_receipt(&rows, Some(HEAD), Some(HASH)).verdict,
            Verdict::NotVerified
        );
    }

    #[test]
    fn missing_git_state_is_not_verified() {
        let rows = [row("cargo test", 0, Some(HEAD), Some(HASH), Some("cli"))];
        assert_eq!(
            build_receipt(&rows, None, Some(HASH)).verdict,
            Verdict::NotVerified
        );
        assert_eq!(
            build_receipt(&rows, Some(HEAD), None).verdict,
            Verdict::NotVerified
        );
    }

    #[test]
    fn text_shows_verdict_command_exit_and_short_commit() {
        let rows = [row("cargo test", 0, Some(HEAD), Some(HASH), Some("cli"))];
        let text = render_text(&build_receipt(&rows, Some(HEAD), Some(HASH)));
        assert!(text.starts_with("Verify receipt: PASSED"));
        assert!(text.contains("command:   cargo test"));
        assert!(text.contains("exit code: 0"));
        assert!(text.contains("commit:    1a2b3c4d5e6f"));
        assert!(!text.contains(HEAD));
    }

    #[test]
    fn not_verified_text_tells_the_user_what_to_run() {
        let text = render_text(&build_receipt(&[], Some(HEAD), Some(HASH)));
        assert!(text.starts_with("Verify receipt: NOT VERIFIED"));
        assert!(text.contains("agent-brain verify -- <cmd>"));
    }

    #[test]
    fn not_a_git_repo_explains_why() {
        let text = render_text(&build_receipt(&[], None, None));
        assert!(text.contains("no git commit found"));
    }

    #[test]
    fn markdown_escapes_pipes_in_the_command() {
        let rows = [row(
            "cargo test -- a|b",
            0,
            Some(HEAD),
            Some(HASH),
            Some("cli"),
        )];
        let markdown = render_markdown(&build_receipt(&rows, Some(HEAD), Some(HASH)));
        assert!(markdown.contains("| Command | `cargo test -- a\\|b` |"));
        assert!(markdown.contains("| Commit | `1a2b3c4d5e6f` |"));
    }

    #[test]
    fn text_and_markdown_show_the_proof_artifact_when_present() {
        let rows = [row("cargo test", 0, Some(HEAD), Some(HASH), Some("cli"))];
        let mut receipt = build_receipt(&rows, Some(HEAD), Some(HASH));
        receipt.evidence.as_mut().unwrap().artifact =
            Some(".verify/artifacts/run-1.log".to_string());
        assert!(render_text(&receipt).contains("artifact:  .verify/artifacts/run-1.log"));
        assert!(render_markdown(&receipt)
            .contains("| Proof artifact | `.verify/artifacts/run-1.log` |"));
    }

    #[test]
    fn no_artifact_line_when_none_was_saved() {
        let rows = [row("cargo test", 0, Some(HEAD), Some(HASH), Some("cli"))];
        let receipt = build_receipt(&rows, Some(HEAD), Some(HASH));
        assert!(!render_text(&receipt).contains("artifact:"));
        assert!(!render_markdown(&receipt).contains("Proof artifact"));
    }

    #[test]
    fn json_uses_snake_case_verdict() {
        let receipt = build_receipt(&[], Some(HEAD), Some(HASH));
        let json = serde_json::to_value(&receipt).expect("serialize");
        assert_eq!(json["verdict"], "not_verified");
        assert!(json["evidence"].is_null());
    }
}
