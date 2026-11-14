use anyhow::Result;
use serde_json::Value;

use crate::tokens::{estimate_json_tokens, estimate_tokens};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TruncatedUpstreamResult {
    pub content: Value,
    pub truncated: bool,
    pub tokens_used: usize,
    pub tokens_budget: usize,
}

pub fn truncate_upstream_result(
    raw: &str,
    structured: Option<&Value>,
    max_tokens: usize,
) -> Result<TruncatedUpstreamResult> {
    let source = structured
        .cloned()
        .or_else(|| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| Value::String(raw.to_string()));

    let (content, truncated) = truncate_value(&source, max_tokens);
    let tokens_used = estimate_json_tokens(&content);
    Ok(TruncatedUpstreamResult {
        content,
        truncated,
        tokens_used,
        tokens_budget: max_tokens,
    })
}

fn truncate_value(value: &Value, max_tokens: usize) -> (Value, bool) {
    if estimate_json_tokens(value) <= max_tokens {
        return (value.clone(), false);
    }

    match value {
        Value::Array(items) => truncate_array(items, max_tokens),
        Value::Object(map) => truncate_object(map, max_tokens),
        Value::String(text) => truncate_text_value(text, max_tokens),
        other => {
            let text = other.to_string();
            let (truncated_text, truncated) = truncate_text(&text, max_tokens);
            (Value::String(truncated_text), truncated)
        }
    }
}

fn compact_entity(val: &Value) -> Option<Value> {
    if let Value::Object(map) = val {
        let mut compact = serde_json::Map::new();
        for key in [
            "id", "uuid", "key", "title", "name", "status", "state", "type",
        ] {
            if let Some(v) = map.get(key) {
                compact.insert(key.to_string(), v.clone());
            }
        }
        if !compact.is_empty() {
            return Some(Value::Object(compact));
        }
    }
    None
}

fn truncate_array(items: &[Value], max_tokens: usize) -> (Value, bool) {
    let mut kept = Vec::new();
    let mut tokens = 0;
    for item in items {
        let item_tokens = estimate_json_tokens(item);
        if tokens + item_tokens > max_tokens {
            if let Some(compact) = compact_entity(item) {
                let compact_tokens = estimate_json_tokens(&compact);
                if tokens + compact_tokens <= max_tokens {
                    tokens += compact_tokens;
                    kept.push(compact);
                    continue;
                }
            }
            if !kept.is_empty() {
                return (
                    Value::Array({
                        let mut out = kept;
                        out.push(Value::String(format!(
                            "... truncated {} more items",
                            items.len() - out.len()
                        )));
                        out
                    }),
                    true,
                );
            }
        }
        if item_tokens > max_tokens {
            let (inner, inner_truncated) = truncate_value(item, max_tokens);
            kept.push(inner);
            return (Value::Array(kept), true || inner_truncated);
        }
        tokens += item_tokens;
        kept.push(item.clone());
    }
    (Value::Array(kept.clone()), kept.len() < items.len())
}

fn truncate_object(map: &serde_json::Map<String, Value>, max_tokens: usize) -> (Value, bool) {
    let mut out = serde_json::Map::new();
    let mut tokens = 0;
    let mut truncated = false;
    for (key, value) in map {
        let entry = serde_json::json!({ key.clone(): value.clone() });
        let entry_tokens = estimate_json_tokens(&entry);
        if tokens + entry_tokens > max_tokens && !out.is_empty() {
            out.insert(
                "_truncated".into(),
                Value::String(format!("{} more fields omitted", map.len() - out.len())),
            );
            return (Value::Object(out), true);
        }
        if entry_tokens > max_tokens {
            let (inner, _) = truncate_value(value, max_tokens.saturating_sub(estimate_tokens(key)));
            out.insert(key.clone(), inner);
            truncated = true;
            break;
        }
        out.insert(key.clone(), value.clone());
        tokens += entry_tokens;
    }
    let out_len = out.len();
    (Value::Object(out), truncated || out_len < map.len())
}

fn truncate_text_value(text: &str, max_tokens: usize) -> (Value, bool) {
    let (truncated, did_truncate) = truncate_text(text, max_tokens);
    (Value::String(truncated), did_truncate)
}

fn truncate_text(text: &str, max_tokens: usize) -> (String, bool) {
    if estimate_tokens(text) <= max_tokens {
        return (text.to_string(), false);
    }

    let max_chars = max_tokens.saturating_mul(4);
    if text.len() <= max_chars {
        return (text.to_string(), false);
    }

    let slice = &text[..max_chars];
    if let Some(pos) = slice.rfind('\n') {
        return (format!("{}\n...[truncated]", &slice[..pos]), true);
    }
    if let Some(pos) = slice.rfind(". ") {
        return (format!("{}.\n...[truncated]", &slice[..=pos]), true);
    }
    if let Some(pos) = slice.rfind(' ') {
        return (format!("{} ...[truncated]", &slice[..pos]), true);
    }
    (format!("{slice}...[truncated]"), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_small_json() {
        let value = json!({"ok": true, "items": [1, 2]});
        let (out, truncated) = truncate_value(&value, 500);
        assert!(!truncated);
        assert_eq!(out, value);
    }

    #[test]
    fn truncates_large_array_without_broken_json() {
        let items: Vec<Value> = (0..200)
            .map(|i| json!({"id": i, "title": format!("issue-{i}")}))
            .collect();
        let value = Value::Array(items);
        let (out, truncated) = truncate_value(&value, 80);
        assert!(truncated);
        assert!(out.is_array());
        serde_json::to_string(&out).expect("valid json");
    }

    #[test]
    fn truncates_plain_text_on_sentence_boundary() {
        let text = "First sentence. Second sentence. Third sentence that keeps going.";
        let (out, truncated) = truncate_text(text, 8);
        assert!(truncated);
        assert!(out.contains("First sentence."));
        assert!(!out.ends_with("going."));
    }
}
