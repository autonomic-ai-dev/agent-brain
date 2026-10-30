use std::collections::HashSet;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::db::store::RecallRow;

pub const SCHEMA_VERSION: u8 = 1;
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 200;
pub const CANDIDATE_CAP: usize = 5000;

pub const USAGE: &str = "\
Usage: agent-brain recall [QUERY WORDS] [--tag TAG ...] [--limit N] [--since 30d|12h|90m] [--semantic] [--min-score F] [--json]

Read stored facts. Tags match a fact's topic or any of its tags.
Without a query, facts come back newest first with score null.
With a query, facts sharing a token with it come back scored by token overlap.
With --semantic, the query is embedded and facts are scored by cosine similarity
against their stored embeddings. Use --min-score to keep only close matches
(for repeat checks, for example `recall \"<topic>\" --tag posted --since 30d --semantic --min-score 0.8`).
Empty results print [] with exit 0. --limit is 1 to 200, default 20.
Conventional tags: voice (writing samples), posted (approved posts), project.";

#[derive(Debug, Clone, PartialEq)]
pub struct RecallQuery {
    pub text: Option<String>,
    pub tags: Vec<String>,
    pub limit: usize,
    pub json: bool,
    pub semantic: bool,
    pub since: Option<Duration>,
    pub min_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecallItem {
    pub schema_version: u8,
    pub id: String,
    pub text: String,
    pub tags: Vec<String>,
    pub created: String,
    pub source: String,
    pub score: Option<f64>,
}

pub fn parse_duration(value: &str) -> Result<Duration> {
    let (digits, unit) = value.split_at(value.len().saturating_sub(1));
    let amount: u64 = digits
        .parse()
        .with_context(|| format!("--since must look like 30d, 12h, or 90m, got `{value}`"))?;
    let seconds = match unit {
        "d" => amount * 86_400,
        "h" => amount * 3_600,
        "m" => amount * 60,
        _ => bail!("--since must look like 30d, 12h, or 90m, got `{value}`"),
    };
    Ok(Duration::from_secs(seconds))
}

pub fn parse_args(args: &[String]) -> Result<RecallQuery> {
    let mut words = Vec::new();
    let mut tags = Vec::new();
    let mut limit = DEFAULT_LIMIT;
    let mut json = false;
    let mut semantic = false;
    let mut since = None;
    let mut min_score = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--tag" => {
                let tag = iter.next().context("--tag needs a value")?;
                tags.push(tag.clone());
            }
            "--limit" => {
                let value = iter.next().context("--limit needs a value")?;
                limit = value
                    .parse::<usize>()
                    .with_context(|| format!("--limit must be a number, got `{value}`"))?;
                if limit == 0 || limit > MAX_LIMIT {
                    bail!("--limit must be between 1 and {MAX_LIMIT}");
                }
            }
            "--since" => {
                let value = iter.next().context("--since needs a value")?;
                since = Some(parse_duration(value)?);
            }
            "--min-score" => {
                let value = iter.next().context("--min-score needs a value")?;
                let parsed: f64 = value
                    .parse()
                    .with_context(|| format!("--min-score must be a number, got `{value}`"))?;
                min_score = Some(parsed);
            }
            "--semantic" => semantic = true,
            "--json" => json = true,
            flag if flag.starts_with("--") => bail!("unknown option `{flag}`"),
            word => words.push(word.to_string()),
        }
    }
    let text = (!words.is_empty()).then(|| words.join(" "));
    if semantic && text.is_none() {
        bail!("--semantic needs a query");
    }
    if min_score.is_some() && !semantic {
        bail!("--min-score needs --semantic");
    }
    Ok(RecallQuery {
        text,
        tags,
        limit,
        json,
        semantic,
        since,
        min_score,
    })
}

pub fn rank_semantic(
    rows: Vec<RecallRow>,
    query_embedding: &[f32],
    min_score: Option<f64>,
    limit: usize,
) -> Vec<RecallItem> {
    let mut scored: Vec<(f64, RecallRow)> = rows
        .into_iter()
        .filter_map(|row| {
            let value = crate::embed::cosine(query_embedding, row.embedding.as_deref()?);
            Some((value, row))
        })
        .filter(|(value, _)| min_score.is_none_or(|floor| *value >= floor))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(value, row)| to_item(row, Some(value)))
        .collect()
}

fn tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

pub fn score(fact: &str, query: &HashSet<String>) -> f64 {
    if query.is_empty() {
        return 0.0;
    }
    let fact_tokens = tokens(fact);
    let shared = query
        .iter()
        .filter(|token| fact_tokens.contains(*token))
        .count();
    shared as f64 / query.len() as f64
}

pub fn rank(rows: Vec<RecallRow>, query: Option<&str>, limit: usize) -> Vec<RecallItem> {
    let query_tokens = query.map(tokens).unwrap_or_default();
    let mut scored: Vec<(f64, RecallRow)> = rows
        .into_iter()
        .map(|row| {
            let value = match query {
                Some(_) => score(&row.fact, &query_tokens),
                None => 0.0,
            };
            (value, row)
        })
        .filter(|(value, _)| query.is_none() || *value > 0.0)
        .collect();
    if query.is_some() {
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    }
    scored
        .into_iter()
        .take(limit)
        .map(|(value, row)| to_item(row, query.is_some().then_some(value)))
        .collect()
}

fn to_item(row: RecallRow, score: Option<f64>) -> RecallItem {
    let created = chrono::DateTime::from_timestamp_millis(row.created_at_ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();
    RecallItem {
        schema_version: SCHEMA_VERSION,
        id: row.id,
        text: row.fact,
        tags: row.tags,
        created,
        source: row.source,
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, topic: &str, fact: &str, updated_at_ms: i64) -> RecallRow {
        RecallRow {
            id: id.to_string(),
            topic: topic.to_string(),
            tags: vec![topic.to_string()],
            fact: fact.to_string(),
            source: "cli".to_string(),
            created_at_ms: updated_at_ms,
            updated_at_ms,
            embedding: None,
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn parses_query_tags_limit_and_json() {
        let query = parse_args(&strings(&[
            "short", "posts", "--tag", "voice", "--tag", "posted", "--limit", "5", "--json",
        ]))
        .unwrap();
        assert_eq!(query.text.as_deref(), Some("short posts"));
        assert_eq!(query.tags, strings(&["voice", "posted"]));
        assert_eq!(query.limit, 5);
        assert!(query.json);
    }

    #[test]
    fn defaults_when_no_options_are_given() {
        let query = parse_args(&[]).unwrap();
        assert_eq!(query.text, None);
        assert!(query.tags.is_empty());
        assert_eq!(query.limit, DEFAULT_LIMIT);
        assert!(!query.json);
    }

    #[test]
    fn rejects_bad_limits_and_unknown_options() {
        assert!(parse_args(&strings(&["--limit", "0"])).is_err());
        assert!(parse_args(&strings(&["--limit", "201"])).is_err());
        assert!(parse_args(&strings(&["--limit", "many"])).is_err());
        assert!(parse_args(&strings(&["--tags", "voice"])).is_err());
        assert!(parse_args(&strings(&["--tag"])).is_err());
    }

    #[test]
    fn empty_input_ranks_to_an_empty_list() {
        assert!(rank(Vec::new(), None, 10).is_empty());
        assert!(rank(Vec::new(), Some("anything"), 10).is_empty());
    }

    #[test]
    fn query_drops_facts_with_no_shared_token_and_orders_by_score() {
        let rows = vec![
            row("a", "voice", "I write short posts about shipping", 1),
            row("b", "voice", "Long essays on architecture", 2),
            row("c", "voice", "Short posts", 3),
        ];
        let items = rank(rows, Some("short posts shipping"), 10);
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
        assert_eq!(items[0].score, Some(1.0));
        assert!((items[1].score.unwrap() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn no_query_keeps_recency_order_and_reports_no_score() {
        let rows = vec![row("new", "posted", "x", 9), row("old", "posted", "y", 1)];
        let items = rank(rows, None, 10);
        assert_eq!(items[0].id, "new");
        assert_eq!(items[0].score, None);
        assert_eq!(items[0].schema_version, SCHEMA_VERSION);
        assert_eq!(items[0].tags, strings(&["posted"]));
    }

    #[test]
    fn parses_since_and_semantic_options() {
        let query = parse_args(&strings(&[
            "short",
            "posts",
            "--semantic",
            "--since",
            "30d",
            "--min-score",
            "0.8",
        ]))
        .unwrap();
        assert!(query.semantic);
        assert_eq!(query.since, Some(Duration::from_secs(30 * 86_400)));
        assert_eq!(query.min_score, Some(0.8));
    }

    #[test]
    fn duration_units_and_bad_values() {
        assert_eq!(
            parse_duration("12h").unwrap(),
            Duration::from_secs(12 * 3_600)
        );
        assert_eq!(parse_duration("90m").unwrap(), Duration::from_secs(90 * 60));
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("30w").is_err());
        assert!(parse_duration("xd").is_err());
    }

    #[test]
    fn semantic_needs_a_query_and_min_score_needs_semantic() {
        assert!(parse_args(&strings(&["--semantic"])).is_err());
        assert!(parse_args(&strings(&["plain", "--min-score", "0.5"])).is_err());
        assert!(parse_args(&strings(&["--since", "soon"])).is_err());
    }

    #[test]
    fn semantic_rank_drops_rows_without_embeddings_and_applies_the_floor() {
        let mut close = row("close", "posted", "a", 1);
        close.embedding = Some(vec![1.0, 0.0]);
        let mut far = row("far", "posted", "b", 2);
        far.embedding = Some(vec![0.0, 1.0]);
        let missing = row("missing", "posted", "c", 3);
        let items = rank_semantic(vec![close, far, missing], &[1.0, 0.0], Some(0.5), 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "close");
        assert!((items[0].score.unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn limit_truncates_the_result() {
        let rows = (0..5)
            .map(|i| row(&i.to_string(), "voice", "shared", i))
            .collect();
        assert_eq!(rank(rows, None, 2).len(), 2);
    }
}
