use serde::{Deserialize, Serialize};

use crate::types::{FileContext, LessonRec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRequest {
    pub paths: Vec<String>,
    pub task_kind: String,
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextPayload {
    File(FileContext),
    Lesson(LessonRec),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextItem {
    pub payload: ContextPayload,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_item_round_trips_through_json() {
        let item = ContextItem {
            payload: ContextPayload::File(FileContext {
                path: "src/engine.rs".into(),
                features: vec![".verify/features/engine.md".into()],
                commands: vec![".verify/run.sh".into()],
            }),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: ContextItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn lesson_item_keeps_its_kind_tag() {
        let item = ContextItem {
            payload: ContextPayload::Lesson(LessonRec {
                paths: vec!["src/a.rs".into()],
                cmd: "cargo test".into(),
                excerpt: "assert failed".into(),
                age_days: 2,
            }),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""kind":"lesson""#), "{json}");
        let back: ContextItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn context_request_keeps_its_paths_in_order() {
        let request = ContextRequest {
            paths: vec!["src/b.rs".into(), "src/a.rs".into()],
            task_kind: "implementing".into(),
            project: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        let back: ContextRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.paths, ["src/b.rs", "src/a.rs"]);
    }
}
