use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct WorkspaceTags {
    pub tags: Vec<String>,
    pub repo_root: Option<String>,
}

pub fn probe(cwd: Option<&Path>) -> WorkspaceTags {
    let Some(cwd) = cwd else {
        return WorkspaceTags::default();
    };

    let repo_root = crate::config::find_repo_root(cwd).map(|p| p.display().to_string());
    let mut tags = Vec::new();

    if cwd.join("package.json").exists() {
        tags.push("node".into());
        if cwd.join("pnpm-lock.yaml").exists() {
            tags.push("pnpm".into());
        } else if cwd.join("yarn.lock").exists() {
            tags.push("yarn".into());
        } else {
            tags.push("npm".into());
        }
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        tags.push("python".into());
    }
    if cwd.join("Cargo.toml").exists() {
        tags.push("rust".into());
    }
    if cwd.join("go.mod").exists() {
        tags.push("go".into());
    }
    if cwd.join("pom.xml").exists() || cwd.join("build.gradle").exists() {
        tags.push("java".into());
    }
    if cwd.join("Gemfile").exists() {
        tags.push("ruby".into());
    }
    if cwd.join("composer.json").exists() {
        tags.push("php".into());
    }

    WorkspaceTags { tags, repo_root }
}

pub fn infer_phase(message: &str) -> String {
    let lower = message.to_lowercase();
    if [
        "review",
        "audit",
        "pr ",
        "pull request",
        "lint",
        "checklist",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return "reviewing".into();
    }
    if [
        "fix", "debug", "error", "bug", "fail", "broken", "crash", "issue",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return "debugging".into();
    }
    if [
        "plan",
        "design",
        "architect",
        "roadmap",
        "spec",
        "blueprint",
        "version",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return "planning".into();
    }
    if [
        "implement",
        "add ",
        "create ",
        "build ",
        "write ",
        "develop",
        "release",
        "deploy",
        "sync",
        "commit",
        "push",
        "mcp",
        "hook",
        "install",
        "test",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return "implementing".into();
    }
    "unknown".into()
}

pub fn infer_task_kind(message: &str) -> crate::types::TaskKind {
    let lower = message.to_lowercase();
    if [
        "verify",
        "verification",
        "test suite",
        "run tests",
        "ci ",
        "proofs",
        "beam",
        "regression",
        "coverage",
        "benchmark",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return crate::types::TaskKind::Verification;
    }
    if [
        "review",
        "audit",
        "pr ",
        "pull request",
        "lint",
        "checklist",
        "inspect",
        "diff",
        "retrospective",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return crate::types::TaskKind::Review;
    }
    if [
        "architect",
        "architecture",
        "roadmap",
        "design doc",
        "system design",
        "blueprint",
        "spec",
        "proposal",
        "rfc",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return crate::types::TaskKind::Architecture;
    }
    if [
        "fix", "debug", "error", "bug", "fail", "broken", "crash", "issue",
        "panic",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        return crate::types::TaskKind::Debugging;
    }
    if ["docker", "compose", "local-dev", "k8s", "kubernetes", "deploy", "infra",
        "setup", "install", "configure", "migration", "sync",
        "docker-compose"]
        .iter()
        .any(|k| lower.contains(k))
    {
        return crate::types::TaskKind::Implementing;
    }
    crate::types::TaskKind::Implementing
}

pub fn is_low_signal_memory(topic: &str, source: Option<&str>) -> bool {
    let topic_lower = topic.to_lowercase();
    topic_lower.starts_with("legacy-")
        || topic_lower.starts_with("legacy_")
        || topic_lower.starts_with("session-digest-")
        || topic_lower.contains("session_digest")
        || matches!(
            source,
            Some("session_digest") | Some("legacy") | Some("legacy_cursor")
        )
}

pub fn agent_boost_keywords(message: &str) -> bool {
    let lower = message.to_lowercase();
    ["review", "debug", "build", "plan", "test", "security"]
        .iter()
        .any(|k| lower.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_phase_covers_operator_workflows() {
        assert_eq!(infer_phase("audit MCP integration"), "reviewing");
        assert_eq!(infer_phase("fix the route gate hook"), "debugging");
        assert_eq!(infer_phase("update the roadmap and VERSIONING"), "planning");
        assert_eq!(infer_phase("implement sync git push"), "implementing");
    }

    #[test]
    fn infer_task_kind_maps_verification() {
        use crate::types::TaskKind;
        assert_eq!(
            infer_task_kind("run BEAM proofs in CI"),
            TaskKind::Verification
        );
        assert_eq!(
            infer_task_kind("implement grpc server"),
            TaskKind::Implementing
        );
    }

    #[test]
    fn low_signal_memory_topics() {
        assert!(is_low_signal_memory("legacy-cursor-de", None));
        assert!(is_low_signal_memory(
            "session-digest-abc",
            Some("session_digest")
        ));
        assert!(!is_low_signal_memory("routing", Some("user")));
    }
}
