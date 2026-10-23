use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;

use crate::config::{find_repo_root, Config};
use crate::db::store::BrainStore;
use crate::types::VerificationStatus;

pub fn gather(config: &Config, cwd: &Path) -> Result<StatusReport> {
    let root = find_repo_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let store = BrainStore::open(&config.db_path)?;
    let last_verification = crate::project_key::project_key_if_known(&root)
        .and_then(|key| crate::verify::last_verification(&store, &key));
    let route_state = config.home.join("hooks").join("route_state.json");
    let last_route_age_secs = std::fs::metadata(&route_state)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|age| age.as_secs());
    Ok(StatusReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        read_gate: crate::route_briefing::read_gate_mode().to_string(),
        allowlist: crate::verify::load_command_allowlist(&root),
        last_verification,
        rule_count: crate::command_rules::load(&crate::command_rules::default_path())?.len(),
        last_route_age_secs,
    })
}

pub struct StatusReport {
    pub version: String,
    pub read_gate: String,
    pub allowlist: Vec<String>,
    pub last_verification: Option<VerificationStatus>,
    pub rule_count: usize,
    pub last_route_age_secs: Option<u64>,
}

impl StatusReport {
    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("agent-brain {}", self.version),
            format!("read gate: {}", self.read_gate),
            format!("routing rules: {}", self.rule_count),
            format!("verify allowlist: {}", self.allowlist.join(", ")),
        ];
        lines.push(match &self.last_verification {
            Some(v) => format!(
                "last verification: {} exit={} ({})",
                v.cmd,
                v.exit_code,
                if v.passing { "passing" } else { "failing" }
            ),
            None => "last verification: none recorded for this repo".to_string(),
        });
        lines.push(match self.last_route_age_secs {
            Some(age) => format!("last route: {age}s ago"),
            None => "last route: none this session".to_string(),
        });
        lines.join("\n") + "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(last_verification: Option<VerificationStatus>) -> StatusReport {
        StatusReport {
            version: "1.0.0".into(),
            read_gate: "hard".into(),
            allowlist: vec!["cargo test".into(), ".verify/run.sh".into()],
            last_verification,
            rule_count: 12,
            last_route_age_secs: Some(90),
        }
    }

    #[test]
    fn status_shows_gate_allowlist_and_latest_verification() {
        let text = report(Some(VerificationStatus {
            cmd: "cargo test --lib".into(),
            exit_code: 0,
            recorded_at: 1,
            passing: true,
        }))
        .render();
        assert!(text.contains("read gate: hard"), "{text}");
        assert!(
            text.contains("verify allowlist: cargo test, .verify/run.sh"),
            "{text}"
        );
        assert!(
            text.contains("last verification: cargo test --lib exit=0 (passing)"),
            "{text}"
        );
        assert!(text.contains("last route: 90s ago"), "{text}");
    }

    #[test]
    fn status_says_when_nothing_was_verified_or_routed() {
        let mut empty = report(None);
        empty.last_route_age_secs = None;
        let text = empty.render();
        assert!(
            text.contains("last verification: none recorded for this repo"),
            "{text}"
        );
        assert!(text.contains("last route: none this session"), "{text}");
    }
}
