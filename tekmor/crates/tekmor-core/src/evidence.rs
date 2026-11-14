use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub cmd: String,
    pub exit_code: i32,
    pub recorded_at: i64,
    pub source: Option<String>,
    pub code_hash: Option<String>,
    pub head_sha: Option<String>,
    pub verify_config_hash: Option<String>,
}

impl EvidenceRecord {
    pub fn is_run_for(&self, head: &str, hash: &str) -> bool {
        self.head_sha.as_deref() == Some(head)
            && self.code_hash.as_deref() == Some(hash)
            && matches!(self.source.as_deref().unwrap_or("cli"), "cli" | "orchestrator")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(source: &str, head: &str, hash: &str) -> EvidenceRecord {
        EvidenceRecord {
            id: "id-1".into(),
            cmd: ".verify/run.sh".into(),
            exit_code: 0,
            recorded_at: 1,
            source: Some(source.into()),
            code_hash: Some(hash.into()),
            head_sha: Some(head.into()),
            verify_config_hash: None,
        }
    }

    #[test]
    fn a_cli_or_orchestrator_run_for_the_same_head_and_tree_counts() {
        assert!(record("cli", "h", "c").is_run_for("h", "c"));
        assert!(record("orchestrator", "h", "c").is_run_for("h", "c"));
    }

    #[test]
    fn a_run_for_another_head_tree_or_source_does_not_count() {
        assert!(!record("cli", "h", "c").is_run_for("other", "c"));
        assert!(!record("cli", "h", "c").is_run_for("h", "other"));
        assert!(!record("hook", "h", "c").is_run_for("h", "c"));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CheckSummary {
    pub cmd: String,
    pub runs: usize,
    pub failures: usize,
    pub last_exit_code: i32,
}

pub fn summarize(records: &[EvidenceRecord]) -> Vec<CheckSummary> {
    let mut by_cmd: std::collections::BTreeMap<&str, Vec<&EvidenceRecord>> =
        std::collections::BTreeMap::new();
    for record in records {
        by_cmd.entry(record.cmd.as_str()).or_default().push(record);
    }
    let mut summaries: Vec<CheckSummary> = by_cmd
        .into_iter()
        .map(|(cmd, runs)| {
            let last = runs.iter().max_by_key(|run| run.recorded_at);
            CheckSummary {
                cmd: cmd.to_string(),
                runs: runs.len(),
                failures: runs.iter().filter(|run| run.exit_code != 0).count(),
                last_exit_code: last.map_or(0, |run| run.exit_code),
            }
        })
        .collect();
    summaries.sort_by(|a, b| b.failures.cmp(&a.failures).then(b.runs.cmp(&a.runs)));
    summaries
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    fn run(cmd: &str, exit_code: i32, recorded_at: i64) -> EvidenceRecord {
        EvidenceRecord {
            id: format!("{cmd}-{recorded_at}"),
            cmd: cmd.into(),
            exit_code,
            recorded_at,
            source: Some("cli".into()),
            code_hash: None,
            head_sha: None,
            verify_config_hash: None,
        }
    }

    #[test]
    fn counts_runs_and_failures_per_check() {
        let summaries = summarize(&[
            run("cargo test", 1, 1),
            run("cargo test", 0, 2),
            run("cargo clippy", 0, 3),
        ]);
        assert_eq!(
            summaries,
            vec![
                CheckSummary { cmd: "cargo test".into(), runs: 2, failures: 1, last_exit_code: 0 },
                CheckSummary { cmd: "cargo clippy".into(), runs: 1, failures: 0, last_exit_code: 0 },
            ]
        );
    }

    #[test]
    fn the_last_result_is_the_most_recent_run_not_the_last_listed() {
        let summaries = summarize(&[run("cargo test", 0, 5), run("cargo test", 2, 3)]);
        assert_eq!(summaries[0].last_exit_code, 0);
    }

    #[test]
    fn the_checks_that_fail_most_come_first() {
        let summaries = summarize(&[
            run("quiet", 0, 1),
            run("flaky", 1, 1),
            run("flaky", 1, 2),
            run("flaky", 0, 3),
        ]);
        assert_eq!(summaries[0].cmd, "flaky");
        assert_eq!(summaries[0].failures, 2);
    }

    #[test]
    fn no_records_gives_no_summaries() {
        assert!(summarize(&[]).is_empty());
    }
}
