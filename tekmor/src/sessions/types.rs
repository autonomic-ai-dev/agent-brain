use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionSource {
    Cursor,
    Codex,
    Gemini,
    OpenCode,
}

impl SessionSource {
    pub const ALL: [SessionSource; 4] = [
        SessionSource::Cursor,
        SessionSource::Codex,
        SessionSource::Gemini,
        SessionSource::OpenCode,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SessionSource::Cursor => "cursor",
            SessionSource::Codex => "codex",
            SessionSource::Gemini => "gemini",
            SessionSource::OpenCode => "opencode",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cursor" => Some(SessionSource::Cursor),
            "codex" => Some(SessionSource::Codex),
            "gemini" | "antigravity" => Some(SessionSource::Gemini),
            "opencode" => Some(SessionSource::OpenCode),
            _ => None,
        }
    }
}

/// One conversation session to digest (JSONL file or OpenCode DB row).
#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub source: SessionSource,
    pub session_id: String,
    pub meta_key: String,
    pub label: String,
    pub jsonl_path: Option<PathBuf>,
    pub opencode_db: Option<PathBuf>,
}

impl SessionTranscript {
    pub fn digest_topic(&self) -> String {
        let short = short_session_slug(&self.session_id);
        format!("session-digest-{}-{short}", self.source.as_str())
    }

    pub fn jsonl(path: PathBuf, source: SessionSource, session_id: String) -> Self {
        let label = path.display().to_string();
        let meta_key = format!("session_digest:{}:{session_id}", source.as_str());
        Self {
            source,
            session_id,
            meta_key,
            label,
            jsonl_path: Some(path),
            opencode_db: None,
        }
    }

    pub fn opencode(db_path: PathBuf, session_id: String) -> Self {
        let label = format!("opencode:{session_id}");
        let meta_key = format!("session_digest:opencode:{session_id}");
        Self {
            source: SessionSource::OpenCode,
            session_id,
            meta_key,
            label,
            jsonl_path: None,
            opencode_db: Some(db_path),
        }
    }
}

pub fn short_session_slug(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(session_id.as_bytes()));
    hash.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_stable_and_short() {
        let a = short_session_slug("c1db6ad4-223d-4e9b-b281-c505a68c6eee");
        assert_eq!(a.len(), 12);
        assert_eq!(
            a,
            short_session_slug("c1db6ad4-223d-4e9b-b281-c505a68c6eee")
        );
    }

    #[test]
    fn digest_topic_includes_source_and_slug() {
        let t = SessionTranscript::jsonl(
            PathBuf::from("/tmp/x.jsonl"),
            SessionSource::Gemini,
            "abc-123".into(),
        );
        assert!(t.digest_topic().starts_with("session-digest-gemini-"));
    }
}
