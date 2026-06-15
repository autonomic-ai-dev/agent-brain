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
    if lower.contains("review") || lower.contains("pr ") {
        return "reviewing".into();
    }
    if lower.contains("fix") || lower.contains("debug") || lower.contains("error") {
        return "debugging".into();
    }
    if lower.contains("plan") || lower.contains("design") || lower.contains("architect") {
        return "planning".into();
    }
    if lower.contains("implement") || lower.contains("add ") || lower.contains("create ") {
        return "implementing".into();
    }
    "unknown".into()
}

pub fn agent_boost_keywords(message: &str) -> bool {
    let lower = message.to_lowercase();
    ["review", "debug", "build", "plan", "test", "security"]
        .iter()
        .any(|k| lower.contains(k))
}
