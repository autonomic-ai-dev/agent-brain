use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::migrations;
use crate::embed::cosine;
use crate::types::{ItemType, ScoredItem};

pub struct BrainStore {
    conn: Arc<Mutex<Connection>>,
    pub index_version: Arc<Mutex<u64>>,
}

impl BrainStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path).context("open brain.db")?;
        migrations::run(&conn).context("run migrations")?;
        let index_version = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'index_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            index_version: Arc::new(Mutex::new(index_version)),
        })
    }

    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let guard = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        f(&guard)
    }

    pub fn bump_index_version(&self) -> Result<u64> {
        let mut ver = self
            .index_version
            .lock()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        *ver += 1;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('index_version', ?1)",
                params![ver.to_string()],
            )?;
            Ok(())
        })?;
        Ok(*ver)
    }

    pub fn get_index_version(&self) -> u64 {
        self.index_version.lock().map(|v| *v).unwrap_or(1)
    }

    pub fn upsert_indexed_item(
        &self,
        item_type: ItemType,
        topic: &str,
        text: &str,
        source_path: &str,
        scope: &str,
        scope_key: Option<&str>,
        content_hash: &str,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let blob = embedding.map(|e| {
            e.iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>()
        });
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM indexed_items WHERE source_path = ?1 AND content_hash != ?2",
                params![source_path, content_hash],
            )?;
            conn.execute(
                r#"INSERT INTO indexed_items (id, item_type, topic, text, source_path, scope, scope_key, content_hash, embedding, updated_at)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                   ON CONFLICT(source_path, content_hash) DO UPDATE SET
                     topic=excluded.topic, text=excluded.text, embedding=excluded.embedding, updated_at=excluded.updated_at"#,
                params![
                    id,
                    item_type.as_str(),
                    topic,
                    text,
                    source_path,
                    scope,
                    scope_key,
                    content_hash,
                    blob,
                    now
                ],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn load_searchable_items(&self) -> Result<Vec<SearchRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT id, item_type, topic, text, source_path, scope, scope_key, embedding
                   FROM indexed_items
                   UNION ALL
                   SELECT id, 'memory', topic, fact, '', scope, scope_key, NULL
                   FROM facts WHERE superseded_by IS NULL AND (expires_at IS NULL OR expires_at > ?1)"#,
            )?;
            let now = chrono::Utc::now().timestamp_millis();
            let rows = stmt
                .query_map(params![now], |row| {
                    Ok(SearchRow {
                        id: row.get(0)?,
                        item_type: row.get(1)?,
                        topic: row.get(2)?,
                        text: row.get(3)?,
                        source_path: row.get(4)?,
                        scope: row.get(5)?,
                        scope_key: row.get(6)?,
                        embedding: row.get(7)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn bm25_search(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT rowid, bm25(items_fts) as score FROM items_fts WHERE items_fts MATCH ?1 ORDER BY score LIMIT ?2",
            )?;
            let q = sanitize_fts_query(query);
            let rows = stmt
                .query_map(params![q, limit as i64], |row| {
                    let rowid: i64 = row.get(0)?;
                    Ok((rowid.to_string(), row.get::<_, f64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn store_fact(
        &self,
        topic: &str,
        fact: &str,
        scope: &str,
        scope_key: Option<&str>,
        confidence: f64,
        content_hash: &str,
        embedding: &[f32],
    ) -> Result<StoreFactResult> {
        let now = chrono::Utc::now().timestamp_millis();
        let expires = now + 90 * 24 * 3600 * 1000;
        let id = Uuid::new_v4().to_string();

        let existing: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM facts WHERE content_hash = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'')",
                    params![content_hash, scope, scope_key],
                    |r| r.get(0),
                )
                .optional()?)
        })?;

        if let Some(existing_id) = existing {
            return Ok(StoreFactResult {
                id: existing_id,
                stored: false,
                deduplicated: true,
            });
        }

        self.with_conn(|conn| {
            if let Some(old_id) = conn
                .query_row(
                    "SELECT id FROM facts WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND superseded_by IS NULL",
                    params![topic, scope, scope_key],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                conn.execute(
                    "UPDATE facts SET superseded_by = ?1 WHERE id = ?2",
                    params![id, old_id],
                )?;
            }

            conn.execute(
                r#"INSERT INTO facts (id, topic, fact, scope, scope_key, source, confidence, created_at, updated_at, expires_at, content_hash)
                   VALUES (?1,?2,?3,?4,?5,'agent',?6,?7,?7,?8,?9)"#,
                params![id, topic, fact, scope, scope_key, confidence, now, expires, content_hash],
            )?;
            Ok(())
        })?;

        let blob: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO indexed_items (id, item_type, topic, text, source_path, scope, scope_key, content_hash, embedding, updated_at)
                   VALUES (?1,'memory',?2,?3,'',?4,?5,?6,?7,?8)
                   ON CONFLICT(source_path, content_hash) DO UPDATE SET text=excluded.text, embedding=excluded.embedding, updated_at=excluded.updated_at"#,
                params![
                    Uuid::new_v4().to_string(),
                    topic,
                    fact,
                    scope,
                    scope_key,
                    content_hash,
                    blob,
                    now
                ],
            )?;
            Ok(())
        })?;

        Ok(StoreFactResult {
            id,
            stored: true,
            deduplicated: false,
        })
    }

    pub fn list_facts(&self, limit: usize) -> Result<Vec<serde_json::Value>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, topic, fact, scope, scope_key, confidence, updated_at FROM facts WHERE superseded_by IS NULL ORDER BY updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "topic": row.get::<_, String>(1)?,
                        "fact": row.get::<_, String>(2)?,
                        "scope": row.get::<_, String>(3)?,
                        "scope_key": row.get::<_, Option<String>>(4)?,
                        "confidence": row.get::<_, f64>(5)?,
                        "updated_at": row.get::<_, i64>(6)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn delete_fact(
        &self,
        id: Option<&str>,
        topic: Option<&str>,
        scope: Option<&str>,
        scope_key: Option<&str>,
    ) -> Result<u64> {
        self.with_conn(|conn| {
            let n = if let Some(id) = id {
                conn.execute("DELETE FROM facts WHERE id = ?1", params![id])?
            } else {
                conn.execute(
                    "DELETE FROM facts WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'')",
                    params![topic.unwrap_or(""), scope.unwrap_or("project"), scope_key],
                )?
            };
            Ok(n as u64)
        })
    }

    pub fn export_facts(&self, export_path: &Path) -> Result<String> {
        let facts = self.list_facts(10_000)?;
        let json = serde_json::to_string_pretty(&facts)?;
        std::fs::write(export_path, &json)?;
        Ok(export_path.display().to_string())
    }

    pub fn score_items(
        &self,
        query: &str,
        query_embedding: &[f32],
        repo_root: Option<&str>,
        tags: &[String],
        boost_agents: bool,
    ) -> Result<Vec<ScoredItem>> {
        let rows = self.load_searchable_items()?;
        let bm25 = self.bm25_search(query, 50).unwrap_or_default();
        let mut bm25_map = std::collections::HashMap::new();
        let mut bm25_max = 0.0f64;
        for (id, score) in bm25 {
            bm25_max = bm25_max.max(score.abs());
            bm25_map.insert(id, score);
        }

        let mut scored = Vec::new();
        for row in rows {
            let item_type = ItemType::parse(&row.item_type).unwrap_or(ItemType::Rule);
            let cosine_sim = if let Some(blob) = &row.embedding {
                let emb = bytes_to_f32(blob);
                cosine(query_embedding, &emb)
            } else {
                0.0
            };

            let bm25_norm = bm25_map
                .get(&row.id)
                .map(|s| if bm25_max > 0.0 { s.abs() / bm25_max } else { 0.0 })
                .unwrap_or(0.0);

            let mut score = 0.7 * cosine_sim + 0.2 * bm25_norm;

            if let Some(root) = repo_root {
                if row.scope_key.as_deref() == Some(root) {
                    score += 0.1;
                }
            }

            for tag in tags {
                if row.topic.to_lowercase().contains(tag)
                    || row.text.to_lowercase().contains(tag)
                {
                    score += 0.15;
                    break;
                }
            }

            if boost_agents && item_type == ItemType::Agent {
                score += 0.10;
            }

            scored.push(ScoredItem {
                id: row.id,
                item_type,
                topic: row.topic,
                text: row.text,
                source_path: if row.source_path.is_empty() {
                    None
                } else {
                    Some(row.source_path)
                },
                scope: row.scope,
                score,
            });
        }

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored)
    }
}

pub struct SearchRow {
    pub id: String,
    pub item_type: String,
    pub topic: String,
    pub text: String,
    pub source_path: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub embedding: Option<Vec<u8>>,
}

pub struct StoreFactResult {
    pub id: String,
    pub stored: bool,
    pub deduplicated: bool,
}

fn bytes_to_f32(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub fn normalize_fact(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn content_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(normalize_fact(text).as_bytes()))
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

pub fn looks_like_secret(text: &str) -> bool {
    let patterns = [
        r"(?i)(api[_-]?key|secret|password|token)\s*[:=]\s*\S+",
        r"sk-[a-zA-Z0-9]{20,}",
        r"ghp_[a-zA-Z0-9]{20,}",
        r"-----BEGIN (RSA |EC )?PRIVATE KEY-----",
    ];
    patterns.iter().any(|p| regex::Regex::new(p).map(|re| re.is_match(text)).unwrap_or(false))
        || text.contains(".env")
        || text.contains(".pem")
}
