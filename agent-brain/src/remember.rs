use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use walkdir::WalkDir;

use crate::db::store::{content_hash, looks_like_secret, BrainStore};

pub const MAX_CHARS: usize = 20_000;
pub const DEFAULT_SOURCE: &str = "cli";
const SCOPE: &str = "project";
const CONFIDENCE: f64 = 0.9;
const POLARITY: &str = "positive";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberRequest {
    pub text: String,
    pub tags: Vec<String>,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    TooLong,
    Secret,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RememberOutcome {
    pub id: String,
    pub deduplicated: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub files: usize,
    pub paragraphs: usize,
    pub stored: usize,
    pub deduplicated: usize,
    pub rejected_secret: usize,
    pub rejected_too_long: usize,
}

pub fn text_rejection(text: &str) -> Option<Rejection> {
    if text.chars().count() > MAX_CHARS {
        return Some(Rejection::TooLong);
    }
    if looks_like_secret(text) {
        return Some(Rejection::Secret);
    }
    None
}

pub fn validate(req: &RememberRequest) -> Result<()> {
    if req.tags.is_empty() {
        bail!("remember needs at least one --tag");
    }
    if req.text.trim().is_empty() {
        bail!("remember needs non-empty --text");
    }
    match text_rejection(&req.text) {
        Some(Rejection::TooLong) => bail!("text exceeds {MAX_CHARS} characters"),
        Some(Rejection::Secret) => bail!("prohibited content: the text looks like a secret"),
        None => Ok(()),
    }
}

pub fn parse_remember_args(args: &[String]) -> Result<RememberRequest> {
    let mut text = None;
    let mut tags = Vec::new();
    let mut source = DEFAULT_SOURCE.to_string();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--text" => text = Some(iter.next().context("--text needs a value")?.clone()),
            "--tag" => tags.push(iter.next().context("--tag needs a value")?.clone()),
            "--source" => source = iter.next().context("--source needs a value")?.clone(),
            other => bail!("unknown argument `{other}`"),
        }
    }
    let req = RememberRequest {
        text: text.context("remember needs --text")?,
        tags,
        source,
    };
    validate(&req)?;
    Ok(req)
}

pub fn remember(
    store: &BrainStore,
    req: &RememberRequest,
    embed: &dyn Fn(&str) -> Result<Vec<f32>>,
) -> Result<RememberOutcome> {
    validate(req)?;
    let topic = &req.tags[0];
    let embedding = embed(&format!("{topic} {}", req.text))?;
    let stored = store.store_fact_full(
        topic,
        &req.text,
        SCOPE,
        None,
        CONFIDENCE,
        &req.source,
        &content_hash(&req.text),
        &embedding,
        POLARITY,
        None,
        None,
    )?;
    store.add_fact_tags(&stored.id, &req.tags)?;
    Ok(RememberOutcome {
        id: stored.id,
        deduplicated: stored.deduplicated,
    })
}

pub fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut current, &mut paragraphs);
        } else {
            current.push(line);
        }
    }
    flush(&mut current, &mut paragraphs);
    paragraphs
}

fn flush(current: &mut Vec<&str>, paragraphs: &mut Vec<String>) {
    if !current.is_empty() {
        paragraphs.push(current.join("\n").trim().to_string());
        current.clear();
    }
}

pub fn import_corpus(
    store: &BrainStore,
    dir: &Path,
    tags: &[String],
    embed: &dyn Fn(&str) -> Result<Vec<f32>>,
) -> Result<ImportReport> {
    if tags.is_empty() {
        bail!("import-corpus needs at least one --tag");
    }
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let mut report = ImportReport::default();
    for entry in WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.context("walk corpus directory")?;
        let is_markdown = entry.path().extension().and_then(|ext| ext.to_str()) == Some("md");
        if !entry.file_type().is_file() || !is_markdown {
            continue;
        }
        report.files += 1;
        let raw = std::fs::read_to_string(entry.path())
            .with_context(|| format!("read {}", entry.path().display()))?;
        for (index, paragraph) in split_paragraphs(&raw).into_iter().enumerate() {
            report.paragraphs += 1;
            match text_rejection(&paragraph) {
                Some(Rejection::Secret) => {
                    report.rejected_secret += 1;
                    continue;
                }
                Some(Rejection::TooLong) => {
                    report.rejected_too_long += 1;
                    continue;
                }
                None => {}
            }
            let req = RememberRequest {
                text: paragraph,
                tags: tags.to_vec(),
                source: format!("{}#paragraph-{}", entry.path().display(), index + 1),
            };
            let outcome = remember(store, &req, embed)?;
            if outcome.deduplicated {
                report.deduplicated += 1;
            } else {
                report.stored += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_blank_lines_and_drops_empty_blocks() {
        let text = "First line\nstill first\n\n\n   \nSecond block\n";
        assert_eq!(
            split_paragraphs(text),
            vec![
                "First line\nstill first".to_string(),
                "Second block".to_string()
            ]
        );
    }

    #[test]
    fn no_paragraphs_from_blank_input() {
        assert!(split_paragraphs("\n  \n\n").is_empty());
    }

    #[test]
    fn secrets_and_oversize_text_are_rejected() {
        assert_eq!(
            text_rejection("api_key = sk-live-123"),
            Some(Rejection::Secret)
        );
        assert_eq!(
            text_rejection(&"a".repeat(MAX_CHARS + 1)),
            Some(Rejection::TooLong)
        );
        assert_eq!(text_rejection("a short, ordinary sentence"), None);
    }

    #[test]
    fn remember_needs_a_tag_and_text() {
        let no_tag = RememberRequest {
            text: "hello".to_string(),
            tags: Vec::new(),
            source: DEFAULT_SOURCE.to_string(),
        };
        assert!(validate(&no_tag).is_err());
        let blank = RememberRequest {
            text: "   ".to_string(),
            tags: vec!["voice".to_string()],
            source: DEFAULT_SOURCE.to_string(),
        };
        assert!(validate(&blank).is_err());
    }

    #[test]
    fn parses_text_repeated_tags_and_source() {
        let args: Vec<String> = [
            "--text",
            "a note",
            "--tag",
            "voice",
            "--tag",
            "posted",
            "--source",
            "https://example.com/post",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let req = parse_remember_args(&args).unwrap();
        assert_eq!(req.text, "a note");
        assert_eq!(req.tags, vec!["voice".to_string(), "posted".to_string()]);
        assert_eq!(req.source, "https://example.com/post");
    }

    #[test]
    fn source_defaults_to_cli() {
        let args: Vec<String> = ["--text", "a note", "--tag", "voice"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_remember_args(&args).unwrap().source, DEFAULT_SOURCE);
    }

    #[test]
    fn rejects_missing_pieces_and_unknown_flags() {
        let no_text: Vec<String> = ["--tag", "voice"].iter().map(|s| s.to_string()).collect();
        assert!(parse_remember_args(&no_text).is_err());
        let no_tag: Vec<String> = ["--text", "a note"].iter().map(|s| s.to_string()).collect();
        assert!(parse_remember_args(&no_tag).is_err());
        let unknown: Vec<String> = ["--text", "a", "--tags", "voice"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(parse_remember_args(&unknown).is_err());
    }

    #[test]
    fn long_text_is_not_capped_by_words() {
        let text = std::iter::repeat("word")
            .take(200)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text_rejection(&text).is_none());
    }
}
