//! Split multi-intent user turns into sub-queries for parallel retrieval.

const MIN_SUBQUERY_LEN: usize = 8;

#[must_use]
pub fn decompose_query(message: &str) -> Vec<String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return vec![String::new()];
    }
    let mut parts = Vec::new();
    for segment in trimmed.split([';', '\n']) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        for piece in split_conjunctions(segment) {
            let piece = piece.trim().trim_matches(|c: char| c == '.' || c == ',');
            if piece.len() >= MIN_SUBQUERY_LEN {
                parts.push(piece.to_string());
            }
        }
    }
    if parts.is_empty() {
        parts.push(trimmed.to_string());
    } else if parts.len() > 1 {
        parts.sort_by_key(|s| std::cmp::Reverse(s.len()));
        parts.dedup();
    }
    parts
}

fn split_conjunctions(segment: &str) -> Vec<String> {
    let lower = segment.to_lowercase();
    for needle in [" and also ", " also ", " plus ", " and then ", " and "] {
        if let Some(idx) = lower.find(needle) {
            let (a, b) = segment.split_at(idx);
            let b = b[needle.len()..].trim();
            let mut out = vec![a.trim().to_string()];
            if !b.is_empty() {
                out.extend(split_conjunctions(b));
            }
            return out;
        }
    }
    vec![segment.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_also() {
        let parts = decompose_query("fix agent-heart CI and also tag v0.6.0 release");
        assert!(parts.len() >= 2);
    }
}
