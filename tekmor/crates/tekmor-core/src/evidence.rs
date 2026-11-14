use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub cmd: String,
    pub exit_code: i32,
    pub recorded_at: i64,
    pub source: String,
    pub code_hash: Option<String>,
    pub head_sha: Option<String>,
}

impl EvidenceRecord {
    pub fn is_run_for(&self, head: &str, hash: &str) -> bool {
        self.head_sha.as_deref() == Some(head)
            && self.code_hash.as_deref() == Some(hash)
            && matches!(self.source.as_str(), "cli" | "orchestrator")
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
            source: source.into(),
            code_hash: Some(hash.into()),
            head_sha: Some(head.into()),
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
        assert!(!record("bench", "h", "c").is_run_for("h", "c"));
    }
}
