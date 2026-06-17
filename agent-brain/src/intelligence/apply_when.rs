//! Runtime matching for fact `apply_when` conditions.

use glob::Pattern;

#[derive(Debug, Clone)]
pub struct MatchContext<'a> {
    pub phase: &'a str,
    pub tags: &'a [String],
    pub open_files: &'a [String],
    pub repo_root: Option<&'a str>,
    pub user_message: &'a str,
}

pub fn parse_apply_when(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| {
        if s.trim().is_empty() {
            return None;
        }
        serde_json::from_str::<Vec<String>>(s).ok()
    })
    .unwrap_or_default()
}

pub fn matches_apply_when(conditions: &[String], ctx: &MatchContext<'_>) -> bool {
    if conditions.is_empty() {
        return true;
    }
    conditions.iter().any(|c| matches_one(c, ctx))
}

fn matches_one(condition: &str, ctx: &MatchContext<'_>) -> bool {
    let Some((kind, value)) = condition.split_once(':') else {
        return false;
    };
    match kind {
        "phase" => ctx.phase.eq_ignore_ascii_case(value),
        "tag" => ctx
            .tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case(value) || t.contains(value)),
        "path" => path_matches(value, ctx),
        _ => false,
    }
}

fn path_matches(pattern_str: &str, ctx: &MatchContext<'_>) -> bool {
    let query = ctx.user_message.to_lowercase();
    if query_mentions_path_glob(pattern_str, &query) {
        return true;
    }
    let Ok(pattern) = Pattern::new(pattern_str) else {
        return false;
    };
    for file in ctx.open_files {
        if pattern.matches(file) {
            return true;
        }
        if let Some(root) = ctx.repo_root {
            let joined = format!("{}/{}", root.trim_end_matches('/'), file.trim_start_matches('/'));
            if pattern.matches(&joined) {
                return true;
            }
        }
    }
    false
}

fn query_mentions_path_glob(pattern_str: &str, query: &str) -> bool {
    let key = pattern_str
        .replace("**", "")
        .replace('*', "")
        .trim_matches('/')
        .to_lowercase();
    if key.is_empty() {
        return false;
    }
    query.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_condition_matches() {
        let ctx = MatchContext {
            phase: "debugging",
            tags: &[],
            open_files: &[],
            repo_root: None,
            user_message: "fix the bug",
        };
        assert!(matches_apply_when(&["phase:debugging".into()], &ctx));
        assert!(!matches_apply_when(&["phase:planning".into()], &ctx));
    }

    #[test]
    fn path_glob_matches_query_mentioning_dist() {
        let ctx = MatchContext {
            phase: "debugging",
            tags: &[],
            open_files: &[],
            repo_root: None,
            user_message: "read frontend build artifacts from dist folder",
        };
        assert!(matches_apply_when(&["path:**/dist/**".into()], &ctx));
    }

    #[test]
    fn path_glob_matches_open_file() {
        let ctx = MatchContext {
            phase: "unknown",
            tags: &[],
            open_files: &["src/components/Button.tsx".into()],
            repo_root: Some("/repo"),
            user_message: "fix button",
        };
        assert!(matches_apply_when(&["path:src/components/**".into()], &ctx));
    }
}
