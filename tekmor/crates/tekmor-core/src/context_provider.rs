use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRequest {
    pub paths: Vec<String>,
    pub task_kind: String,
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: String,
    pub kind: String,
    pub token_cost: usize,
    pub why_now: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_item_round_trips_through_json() {
        let item = ContextItem {
            id: "lesson-42".into(),
            kind: "lesson".into(),
            token_cost: 120,
            why_now: "a failing test touched src/engine.rs".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
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
