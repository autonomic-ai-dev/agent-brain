pub fn estimate_tokens(text: &str) -> usize {
    // cl100k_base approximation: ~4 chars per token + safety margin
    ((text.len() as f64) / 4.0).ceil() as usize + 1
}

pub fn estimate_json_tokens(value: &serde_json::Value) -> usize {
    estimate_tokens(&value.to_string())
}
