//! Temporal knowledge graph helpers for ADD-only memory (Zep-inspired validity windows).

use anyhow::Result;
use rusqlite::{params, Connection};
use uuid::Uuid;

/// A memory fact is active when `valid_from <= now` and `invalid_at` is unset or in the future.
pub fn is_fact_active(now_ms: i64, valid_from: i64, invalid_at: Option<i64>) -> bool {
    if valid_from > now_ms {
        return false;
    }
    match invalid_at {
        Some(until) => until > now_ms,
        None => true,
    }
}

/// SQL fragment for active-fact filtering; pass the bind placeholder for `now` (e.g. `"?1"`).
pub fn active_fact_sql(now_param: &str) -> String {
    format!(
        "(valid_from IS NULL OR valid_from <= {now_param}) AND (invalid_at IS NULL OR invalid_at > {now_param})"
    )
}

/// Archive facts whose validity window has ended and prune stale KG edges.
pub fn prune_expired(conn: &Connection, now_ms: i64) -> Result<PruneReport> {
    let mut stmt = conn.prepare(
        r#"SELECT id, topic, fact, scope, scope_key, source, confidence, polarity, apply_when
         FROM facts
         WHERE superseded_by IS NULL
           AND invalid_at IS NOT NULL
           AND invalid_at <= ?1
           AND id NOT IN (SELECT original_id FROM facts_archive)"#,
    )?;
    type ExpiredFact = (
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        f64,
        String,
        Option<String>,
    );
    let rows: Vec<ExpiredFact> = stmt
        .query_map(params![now_ms], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut archived_facts = 0usize;
    for (id, topic, fact, scope, scope_key, source, confidence, polarity, apply_when) in rows {
        conn.execute(
            r#"INSERT OR IGNORE INTO facts_archive
               (id, original_id, topic, fact, scope, scope_key, source, confidence, polarity, apply_when, archived_at, archive_reason)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'temporal_invalid')"#,
            params![
                Uuid::new_v4().to_string(),
                id,
                topic,
                fact,
                scope,
                scope_key,
                source,
                confidence,
                polarity,
                apply_when,
                now_ms
            ],
        )?;
        conn.execute("DELETE FROM facts WHERE id = ?1", params![id])?;
        archived_facts += 1;
    }

    let pruned_edges = conn.execute(
        "DELETE FROM memory_kg_edges WHERE invalid_at IS NOT NULL AND invalid_at <= ?1",
        params![now_ms],
    )?;

    Ok(PruneReport {
        archived_facts,
        pruned_edges,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneReport {
    pub archived_facts: usize,
    pub pruned_edges: usize,
}

/// Link a new fact to a prior fact on the same topic (ADD-only evolution edge).
pub fn link_fact_evolution(
    conn: &Connection,
    source_fact_id: &str,
    target_fact_id: &str,
    relation: &str,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        r#"INSERT INTO memory_kg_edges (id, source_fact_id, target_fact_id, relation, valid_from, invalid_at, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?5)"#,
        params![
            Uuid::new_v4().to_string(),
            source_fact_id,
            target_fact_id,
            relation,
            now_ms
        ],
    )?;
    Ok(())
}

/// Facts reachable within `max_hops` from a seed fact (recursive CTE on memory_kg_edges).
pub fn related_fact_ids(
    conn: &Connection,
    seed_fact_id: &str,
    max_hops: u32,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        r#"WITH RECURSIVE walk(fact_id, depth) AS (
               SELECT ?1, 0
               UNION ALL
               SELECT e.target_fact_id, walk.depth + 1
               FROM memory_kg_edges e
               JOIN walk ON e.source_fact_id = walk.fact_id
               WHERE walk.depth < ?2
                 AND (e.invalid_at IS NULL OR e.invalid_at > ?3)
           )
           SELECT DISTINCT fact_id FROM walk WHERE depth > 0"#,
    )?;
    let now = chrono::Utc::now().timestamp_millis();
    let ids = stmt
        .query_map(params![seed_fact_id, max_hops, now], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn active_fact_respects_validity_window() {
        let now = 1_000_000i64;
        assert!(is_fact_active(now, 900_000, None));
        assert!(is_fact_active(now, 900_000, Some(1_100_000)));
        assert!(!is_fact_active(now, 1_100_000, None));
        assert!(!is_fact_active(now, 900_000, Some(900_000)));
    }

    #[test]
    fn prune_archives_invalid_facts() {
        let conn = mem_db();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            r#"INSERT INTO facts (id, topic, fact, scope, scope_key, source, confidence, created_at, updated_at, expires_at, content_hash, polarity, valid_from, invalid_at)
               VALUES ('old', 'city', 'NYC', 'global', NULL, 'agent', 0.9, ?1, ?1, NULL, 'h1', 'positive', ?1, ?2)"#,
            params![now - 10_000, now - 1],
        )
        .unwrap();
        let report = prune_expired(&conn, now).unwrap();
        assert_eq!(report.archived_facts, 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM facts WHERE id = 'old'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn kg_traversal_finds_evolved_facts() {
        let conn = mem_db();
        let now = chrono::Utc::now().timestamp_millis();
        for (id, fact) in [("f1", "NYC"), ("f2", "SF")] {
            conn.execute(
                r#"INSERT INTO facts (id, topic, fact, scope, scope_key, source, confidence, created_at, updated_at, expires_at, content_hash, polarity, valid_from)
                   VALUES (?1, 'city', ?2, 'global', NULL, 'agent', 0.9, ?3, ?3, NULL, ?1, 'positive', ?3)"#,
                params![id, fact, now],
            )
            .unwrap();
        }
        link_fact_evolution(&conn, "f2", "f1", "evolved_from", now).unwrap();
        let related = related_fact_ids(&conn, "f2", 2).unwrap();
        assert!(related.contains(&"f1".to_string()));
    }
}
