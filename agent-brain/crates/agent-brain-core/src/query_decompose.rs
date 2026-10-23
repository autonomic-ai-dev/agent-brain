//! Split multi-intent user turns into sub-queries for parallel retrieval.

const MIN_SUBQUERY_LEN: usize = 12;

/// Strong multi-intent delimiters only — avoid splitting natural "X and Y" skill queries.
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
        for piece in split_explicit_conjunctions(segment) {
            let piece = piece.trim().trim_matches(|c: char| c == '.' || c == ',');
            if piece.len() >= MIN_SUBQUERY_LEN {
                parts.push(piece.to_string());
            }
        }
    }

    if parts.len() <= 1 {
        return vec![trimmed.to_string()];
    }

    parts.sort_by_key(|s| std::cmp::Reverse(s.len()));
    parts.dedup();
    parts
}

fn split_explicit_conjunctions(segment: &str) -> Vec<String> {
    let lower = segment.to_lowercase();
    for needle in [" and also ", " and then ", " plus "] {
        if let Some(idx) = lower.find(needle) {
            let (a, b) = segment.split_at(idx);
            let b = b[needle.len()..].trim();
            let mut out = vec![a.trim().to_string()];
            if !b.is_empty() {
                out.extend(split_explicit_conjunctions(b));
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

    #[test]
    fn keeps_natural_and_phrase_intact() {
        let msg = "optimize react rendering and memoization patterns";
        let parts = decompose_query(msg);
        assert_eq!(parts, vec![msg.to_string()]);
    }
}
