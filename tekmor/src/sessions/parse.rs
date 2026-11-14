use super::types::SessionSource;

/// Parse one JSONL line into user-visible text (source-aware).
pub fn parse_user_line(source: SessionSource, line: &str) -> Option<String> {
    match source {
        SessionSource::Cursor | SessionSource::Codex => super::extract_user_text(line),
        SessionSource::Gemini => parse_gemini_line(line),
        SessionSource::OpenCode => None,
    }
}

fn parse_gemini_line(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let typ = v.get("type").and_then(|t| t.as_str())?;
    if typ != "USER_INPUT" {
        return None;
    }
    let content = v.get("content").and_then(|c| c.as_str())?;
    let text =
        extract_xml_tag(content, "USER_REQUEST").unwrap_or_else(|| content.trim().to_string());
    let cleaned = strip_user_query_tags(&text).trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn extract_xml_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_string())
}

fn strip_user_query_tags(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(start) = out.find("<user_query>") {
        if let Some(end) = out.find("</user_query>") {
            let inner = &out[start + 12..end];
            return inner.trim().to_string();
        }
    }
    for tag in [
        "<ADDITIONAL_METADATA>",
        "<USER_SETTINGS_CHANGE>",
        "<manually_attached_skills>",
        "<user_info>",
        "<rules>",
        "<agent_skills>",
    ] {
        if let Some(pos) = out.find(tag) {
            out = out[..pos].to_string();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gemini_user_request() {
        let line = r#"{"type":"USER_INPUT","content":"<USER_REQUEST>\nuse vitest not jest\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nignored\n</ADDITIONAL_METADATA>"}"#;
        let text = parse_user_line(SessionSource::Gemini, line).unwrap();
        assert!(text.contains("vitest"));
        assert!(!text.contains("ADDITIONAL_METADATA"));
    }

    #[test]
    fn ignores_non_user_input_gemini_lines() {
        let line = r#"{"type":"AGENT_OUTPUT","content":"done"}"#;
        assert!(parse_user_line(SessionSource::Gemini, line).is_none());
    }
}
