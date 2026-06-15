#![allow(clippy::too_many_arguments)] // row-oriented SQLite APIs take many optional columns

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::migrations;
use crate::embed::{batch_dot_products, normalize_embedding};
use crate::intelligence::{parse_apply_when, MatchContext, matches_apply_when};
use crate::types::{ItemType, ScoredItem};

const BM25_ITEMS_TOP: usize = 150;
const BM25_FACTS_TOP: usize = 40;
const MIN_CANDIDATES: usize = 50;
const FALLBACK_RECENT: usize = 80;
const QUERY_EMBEDDING_CACHE_MAX: usize = 500;
const BM25_FAST_PATH_MIN_STRENGTH: f64 = 2.5;

pub(crate) struct SearchIndexCache {
    version: u64,
    indexed: Vec<CachedRow>,
    indexed_by_id: HashMap<String, usize>,
    global_indices: Vec<usize>,
    scoped_indices: HashMap<String, Vec<usize>>,
    memories: Vec<CachedRow>,
    memories_by_id: HashMap<String, usize>,
}

pub(crate) struct Bm25Prefilter {
    item_ids: HashSet<String>,
    memory_ids: HashSet<String>,
    bm25_map: HashMap<String, f64>,
    bm25_max: f64,
}

impl Bm25Prefilter {
    pub(crate) fn fast_path_eligible(&self) -> bool {
        !self.item_ids.is_empty() && self.bm25_max >= BM25_FAST_PATH_MIN_STRENGTH
    }
}

#[derive(Clone)]
struct CachedRow {
    id: String,
    item_type: String,
    topic: String,
    text: String,
    source_path: String,
    scope: String,
    scope_key: Option<String>,
    embedding: Option<Vec<f32>>,
    updated_at: i64,
    polarity: Option<String>,
    source: Option<String>,
    confidence: f64,
    apply_when: Option<String>,
}

pub struct BrainStore {
    conn: Arc<Mutex<Connection>>,
    pub index_version: Arc<Mutex<u64>>,
    search_cache: Mutex<Option<Arc<SearchIndexCache>>>,
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
            search_cache: Mutex::new(None),
        })
    }

    fn invalidate_search_cache(&self) {
        if let Ok(mut guard) = self.search_cache.lock() {
            *guard = None;
        }
    }

    pub(crate) fn search_cache_snapshot(&self) -> Result<Arc<SearchIndexCache>> {
        let version = self.get_index_version();
        if let Ok(guard) = self.search_cache.lock() {
            if let Some(cache) = guard.as_ref() {
                if cache.version == version {
                    return Ok(Arc::clone(cache));
                }
            }
        }

        let indexed = self.load_indexed_rows()?;
        let memories = self.load_memory_rows()?;
        let indexed_by_id: HashMap<String, usize> = indexed
            .iter()
            .enumerate()
            .map(|(i, row)| (row.id.clone(), i))
            .collect();
        let mut global_indices = Vec::new();
        let mut scoped_indices: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, row) in indexed.iter().enumerate() {
            if row.scope == "global" || row.scope_key.is_none() {
                global_indices.push(i);
            } else if let Some(key) = &row.scope_key {
                scoped_indices.entry(key.clone()).or_default().push(i);
            }
        }
        let memories_by_id: HashMap<String, usize> = memories
            .iter()
            .enumerate()
            .map(|(i, row)| (row.id.clone(), i))
            .collect();
        let cache = Arc::new(SearchIndexCache {
            version,
            indexed,
            indexed_by_id,
            global_indices,
            scoped_indices,
            memories,
            memories_by_id,
        });
        if let Ok(mut guard) = self.search_cache.lock() {
            *guard = Some(Arc::clone(&cache));
        }
        Ok(cache)
    }

    pub fn prewarm_search_cache(&self) -> Result<()> {
        let _ = self.search_cache_snapshot()?;
        Ok(())
    }

    fn load_indexed_rows(&self) -> Result<Vec<CachedRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT id, item_type, topic, text, source_path, scope, scope_key, embedding, updated_at
                   FROM indexed_items"#,
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(CachedRow {
                        id: row.get(0)?,
                        item_type: row.get(1)?,
                        topic: row.get(2)?,
                        text: row.get(3)?,
                        source_path: row.get(4)?,
                        scope: row.get(5)?,
                        scope_key: row.get(6)?,
                        embedding: row
                            .get::<_, Option<Vec<u8>>>(7)?
                            .map(|b| normalize_embedding(bytes_to_f32(&b))),
                        updated_at: row.get(8)?,
                        polarity: None,
                        source: None,
                        confidence: 0.9,
                        apply_when: None,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    fn load_memory_rows(&self) -> Result<Vec<CachedRow>> {
        self.with_conn(|conn| {
            let now = chrono::Utc::now().timestamp_millis();
            let mut stmt = conn.prepare(
                r#"SELECT id, topic, fact, scope, scope_key, updated_at, polarity, source, confidence, apply_when
                   FROM facts WHERE superseded_by IS NULL AND (expires_at IS NULL OR expires_at > ?1)"#,
            )?;
            let rows = stmt
                .query_map(params![now], |row| {
                    Ok(CachedRow {
                        id: row.get(0)?,
                        item_type: "memory".into(),
                        topic: row.get(1)?,
                        text: row.get(2)?,
                        source_path: String::new(),
                        scope: row.get(3)?,
                        scope_key: row.get(4)?,
                        embedding: None,
                        updated_at: row.get(5)?,
                        polarity: row.get(6)?,
                        source: row.get(7)?,
                        confidence: row.get(8)?,
                        apply_when: row.get(9)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
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
        self.invalidate_search_cache();
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

    pub fn indexed_item_current_hash(&self, source_path: &str) -> Result<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT content_hash FROM indexed_items WHERE source_path = ?1 AND embedding IS NOT NULL LIMIT 1",
                params![source_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
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
            let normalized = normalize_embedding(e.to_vec());
            normalized
                .iter()
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

    pub fn delete_indexed_items_under_prefix(&self, path_prefix: &str) -> Result<u64> {
        let prefix = path_prefix.trim_end_matches('/');
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM indexed_items WHERE source_path = ?1 OR source_path LIKE ?2",
                params![prefix, format!("{prefix}/%")],
            )?;
            Ok(n as u64)
        })
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

    pub fn bm25_search_items(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT i.id, bm25(items_fts) AS score
                   FROM items_fts
                   JOIN indexed_items i ON i.rowid = items_fts.rowid
                   WHERE items_fts MATCH ?1
                   ORDER BY score
                   LIMIT ?2"#,
            )?;
            let q = sanitize_fts_query(query);
            let rows = stmt
                .query_map(params![q, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn bm25_search_facts(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        self.with_conn(|conn| {
            let now = chrono::Utc::now().timestamp_millis();
            let mut stmt = conn.prepare(
                r#"SELECT f.id, bm25(facts_fts) AS score
                   FROM facts_fts
                   JOIN facts f ON f.rowid = facts_fts.rowid
                   WHERE facts_fts MATCH ?1
                     AND f.superseded_by IS NULL
                     AND (f.expires_at IS NULL OR f.expires_at > ?2)
                   ORDER BY score
                   LIMIT ?3"#,
            )?;
            let q = sanitize_fts_query(query);
            let rows = stmt
                .query_map(params![q, now, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    #[allow(dead_code)]
    pub fn bm25_search(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        self.bm25_search_items(query, limit)
    }

    pub fn get_query_embedding(&self, query_hash: &str) -> Result<Option<Vec<f32>>> {
        self.with_conn(|conn| {
            let blob: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT embedding FROM query_embeddings WHERE query_hash = ?1",
                    params![query_hash],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(blob.map(|b| normalize_embedding(bytes_to_f32(&b))))
        })
    }

    pub fn put_query_embedding(&self, query_hash: &str, embedding: &[f32]) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let normalized = normalize_embedding(embedding.to_vec());
        let blob: Vec<u8> = normalized.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO query_embeddings (query_hash, embedding, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(query_hash) DO UPDATE SET embedding = excluded.embedding, updated_at = excluded.updated_at",
                params![query_hash, blob, now],
            )?;
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM query_embeddings", [], |r| r.get(0))?;
            if count as usize > QUERY_EMBEDDING_CACHE_MAX {
                let excess = count as usize - QUERY_EMBEDDING_CACHE_MAX;
                conn.execute(
                    "DELETE FROM query_embeddings WHERE query_hash IN (
                        SELECT query_hash FROM query_embeddings ORDER BY updated_at ASC LIMIT ?1
                    )",
                    params![excess as i64],
                )?;
            }
            Ok(())
        })
    }

    pub fn ensure_embedding_model(&self, model_id: &str) -> Result<bool> {
        let current = self.get_meta("embedding_model")?;
        if current.as_deref() == Some(model_id) {
            return Ok(false);
        }
        if current.is_none() {
            self.set_meta("embedding_model", model_id)?;
            return Ok(false);
        }
        self.with_conn(|conn| {
            conn.execute("UPDATE indexed_items SET embedding = NULL", [])?;
            conn.execute("DELETE FROM query_embeddings", [])?;
            Ok(())
        })?;
        self.set_meta("embedding_model", model_id)?;
        self.invalidate_search_cache();
        self.bump_index_version()?;
        tracing::info!(
            target: "agent_brain::index",
            model = model_id,
            "embedding model changed; cleared cached vectors for re-index"
        );
        Ok(true)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
            Ok(())
        })
    }

    pub fn store_fact(
        &self,
        topic: &str,
        fact: &str,
        scope: &str,
        scope_key: Option<&str>,
        confidence: f64,
        source: &str,
        content_hash: &str,
        embedding: &[f32],
        polarity: &str,
    ) -> Result<StoreFactResult> {
        self.store_fact_full(
            topic,
            fact,
            scope,
            scope_key,
            confidence,
            source,
            content_hash,
            embedding,
            polarity,
            None,
        )
    }

    pub fn store_fact_full(
        &self,
        topic: &str,
        fact: &str,
        scope: &str,
        scope_key: Option<&str>,
        confidence: f64,
        source: &str,
        content_hash: &str,
        embedding: &[f32],
        polarity: &str,
        apply_when: Option<&str>,
    ) -> Result<StoreFactResult> {
        let polarity = if polarity == "negative" {
            "negative"
        } else {
            "positive"
        };
        let now = chrono::Utc::now().timestamp_millis();
        let expires = now + 90 * 24 * 3600 * 1000;
        let id = Uuid::new_v4().to_string();

        let existing: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM facts WHERE content_hash = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND superseded_by IS NULL",
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

        let superseded: Option<(String, String)> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id, fact FROM facts WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND superseded_by IS NULL",
                    params![topic, scope, scope_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
        })?;

        self.with_conn(|conn| {
            if let Some((old_id, old_fact)) = &superseded {
                conn.execute(
                    "UPDATE facts SET superseded_by = ?1 WHERE id = ?2",
                    params![id, old_id],
                )?;
                let conflict_id = Uuid::new_v4().to_string();
                conn.execute(
                    r#"INSERT INTO conflict_log (id, timestamp, sync_source, topic, scope, scope_key, loser_id, loser_fact, winner_id, winner_fact, resolution)
                       VALUES (?1,?2,'local',?3,?4,?5,?6,?7,?8,?9,'superseded')"#,
                    params![
                        conflict_id,
                        now,
                        topic,
                        scope,
                        scope_key,
                        old_id,
                        old_fact,
                        id,
                        fact
                    ],
                )?;
            }

            conn.execute(
                r#"INSERT INTO facts (id, topic, fact, scope, scope_key, source, confidence, created_at, updated_at, expires_at, content_hash, polarity, apply_when)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8,?9,?10,?11,?12)"#,
                params![id, topic, fact, scope, scope_key, source, confidence, now, expires, content_hash, polarity, apply_when],
            )?;
            Ok(())
        })?;

        let blob: Vec<u8> = normalize_embedding(embedding.to_vec())
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
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

        self.invalidate_search_cache();
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

    pub fn ensure_device_id(&self) -> Result<String> {
        if let Some(id) = self.get_meta("device_id")? {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        self.set_meta("device_id", &id)?;
        Ok(id)
    }

    pub fn list_export_facts(&self) -> Result<Vec<serde_json::Value>> {
        self.with_conn(|conn| {
            let now = chrono::Utc::now().timestamp_millis();
            let mut stmt = conn.prepare(
                r#"SELECT id, topic, fact, scope, scope_key, source, confidence, polarity, apply_when, content_hash, created_at, updated_at
                   FROM facts WHERE superseded_by IS NULL AND (expires_at IS NULL OR expires_at > ?1)"#,
            )?;
            let rows = stmt
                .query_map(params![now], |row| {
                    let apply_raw: Option<String> = row.get(8)?;
                    let apply_when = apply_raw
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok());
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "topic": row.get::<_, String>(1)?,
                        "fact": row.get::<_, String>(2)?,
                        "scope": row.get::<_, String>(3)?,
                        "scope_key": row.get::<_, Option<String>>(4)?,
                        "source": row.get::<_, String>(5)?,
                        "confidence": row.get::<_, f64>(6)?,
                        "polarity": row.get::<_, String>(7)?,
                        "apply_when": apply_when,
                        "content_hash": row.get::<_, String>(9)?,
                        "created_at": row.get::<_, i64>(10)?,
                        "updated_at": row.get::<_, i64>(11)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn fact_exists_by_hash(
        &self,
        content_hash: &str,
        scope: &str,
        scope_key: Option<&str>,
    ) -> Result<bool> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM facts WHERE content_hash = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND superseded_by IS NULL LIMIT 1",
                    params![content_hash, scope, scope_key],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })
    }

    pub fn get_active_fact_by_topic(
        &self,
        topic: &str,
        scope: &str,
        scope_key: Option<&str>,
    ) -> Result<Option<ActiveFactSnapshot>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, fact, updated_at FROM facts WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND superseded_by IS NULL",
                params![topic, scope, scope_key],
                |row| {
                    Ok(ActiveFactSnapshot {
                        id: row.get(0)?,
                        fact: row.get(1)?,
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn log_import_conflict(
        &self,
        sync_source: &str,
        topic: &str,
        scope: &str,
        scope_key: Option<&str>,
        loser_id: &str,
        loser_fact: &str,
        winner_id: &str,
        winner_fact: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conflict_id = Uuid::new_v4().to_string();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO conflict_log (id, timestamp, sync_source, topic, scope, scope_key, loser_id, loser_fact, winner_id, winner_fact, resolution)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'newer_updated_at')"#,
                params![
                    conflict_id,
                    now,
                    sync_source,
                    topic,
                    scope,
                    scope_key,
                    loser_id,
                    loser_fact,
                    winner_id,
                    winner_fact
                ],
            )?;
            Ok(())
        })
    }

    pub fn scope_conflict_warnings(&self, topics: &[String]) -> Result<Vec<(String, String)>> {
        let mut warnings = Vec::new();
        for topic in topics {
            let rows: Vec<(String, String)> = self.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT scope, fact FROM facts WHERE topic = ?1 AND superseded_by IS NULL",
                )?;
                let rows = stmt
                    .query_map(params![topic], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })?;
            if rows.len() < 2 {
                continue;
            }
            let global = rows.iter().find(|(s, _)| s == "global");
            let project = rows.iter().find(|(s, _)| s == "project");
            if let (Some((_, g)), Some((_, p))) = (global, project) {
                if g != p {
                    warnings.push((
                        topic.clone(),
                        format!("Global vs project conflict on '{topic}'"),
                    ));
                }
            }
        }
        Ok(warnings)
    }

    pub fn get_fact(&self, id: &str) -> Result<Option<serde_json::Value>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, topic, fact, scope, scope_key, confidence, polarity, updated_at FROM facts WHERE id = ?1",
                params![id],
                |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "topic": row.get::<_, String>(1)?,
                        "fact": row.get::<_, String>(2)?,
                        "scope": row.get::<_, String>(3)?,
                        "scope_key": row.get::<_, Option<String>>(4)?,
                        "confidence": row.get::<_, f64>(5)?,
                        "polarity": row.get::<_, String>(6)?,
                        "updated_at": row.get::<_, i64>(7)?,
                    }))
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn insert_retrieval_log(
        &self,
        id: &str,
        query_hash: &str,
        phase: &str,
        items_json: &str,
        tokens_used: usize,
        truncated: bool,
        cache_hit: bool,
        latency_ms: u64,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO retrieval_log (id, timestamp, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"#,
                params![
                    id,
                    now,
                    query_hash,
                    phase,
                    items_json,
                    tokens_used as i64,
                    truncated as i64,
                    cache_hit as i64,
                    latency_ms as i64
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_retrieval_log(&self, id: &str) -> Result<Option<RetrievalLogRow>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms FROM retrieval_log WHERE id = ?1",
                params![id],
                |row| {
                    Ok(RetrievalLogRow {
                        id: row.get(0)?,
                        query_hash: row.get(1)?,
                        phase: row.get(2)?,
                        items_json: row.get(3)?,
                        tokens_used: row.get::<_, i64>(4)? as usize,
                        truncated: row.get::<_, i64>(5)? != 0,
                        cache_hit: row.get::<_, i64>(6)? != 0,
                        latency_ms: row.get::<_, i64>(7)? as u64,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn latest_retrieval_log(&self) -> Result<Option<RetrievalLogRow>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms FROM retrieval_log ORDER BY timestamp DESC LIMIT 1",
                [],
                |row| {
                    Ok(RetrievalLogRow {
                        id: row.get(0)?,
                        query_hash: row.get(1)?,
                        phase: row.get(2)?,
                        items_json: row.get(3)?,
                        tokens_used: row.get::<_, i64>(4)? as usize,
                        truncated: row.get::<_, i64>(5)? != 0,
                        cache_hit: row.get::<_, i64>(6)? != 0,
                        latency_ms: row.get::<_, i64>(7)? as u64,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn list_retrieval_logs(&self, limit: usize) -> Result<Vec<RetrievalLogRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms FROM retrieval_log ORDER BY timestamp DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(RetrievalLogRow {
                        id: row.get(0)?,
                        query_hash: row.get(1)?,
                        phase: row.get(2)?,
                        items_json: row.get(3)?,
                        tokens_used: row.get::<_, i64>(4)? as usize,
                        truncated: row.get::<_, i64>(5)? != 0,
                        cache_hit: row.get::<_, i64>(6)? != 0,
                        latency_ms: row.get::<_, i64>(7)? as u64,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn list_conflicts(&self, limit: usize) -> Result<Vec<serde_json::Value>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, sync_source, topic, scope, scope_key, loser_id, loser_fact, winner_id, winner_fact, resolution, restored FROM conflict_log ORDER BY timestamp DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "timestamp": row.get::<_, i64>(1)?,
                        "sync_source": row.get::<_, String>(2)?,
                        "topic": row.get::<_, String>(3)?,
                        "scope": row.get::<_, String>(4)?,
                        "scope_key": row.get::<_, Option<String>>(5)?,
                        "loser_id": row.get::<_, String>(6)?,
                        "loser_fact": row.get::<_, String>(7)?,
                        "winner_id": row.get::<_, String>(8)?,
                        "winner_fact": row.get::<_, String>(9)?,
                        "resolution": row.get::<_, String>(10)?,
                        "restored": row.get::<_, i64>(11)? != 0,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn get_conflict(&self, id: &str) -> Result<Option<ConflictSnapshot>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, topic, scope, scope_key, loser_id, loser_fact, winner_id, restored FROM conflict_log WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ConflictSnapshot {
                        id: row.get(0)?,
                        topic: row.get(1)?,
                        scope: row.get(2)?,
                        scope_key: row.get(3)?,
                        loser_id: row.get(4)?,
                        loser_fact: row.get(5)?,
                        winner_id: row.get(6)?,
                        restored: row.get::<_, i64>(7)? != 0,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn delete_fact_by_id(&self, id: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM facts WHERE id = ?1", rusqlite::params![id])?;
            Ok(())
        })
    }

    pub fn mark_conflict_restored(&self, id: &str) -> Result<()> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE conflict_log SET restored = 1 WHERE id = ?1",
                params![id],
            )?;
            if updated == 0 {
                anyhow::bail!("conflict not found: {id}");
            }
            Ok(())
        })
    }

    pub fn count_unresolved_conflicts(&self) -> Result<usize> {
        self.with_conn(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM conflict_log WHERE restored = 0",
                [],
                |r| r.get(0),
            )?;
            Ok(count as usize)
        })
    }

    pub fn record_context_feedback(&self, item_ids: &[String], useful: bool) -> Result<usize> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut updated = 0usize;
        self.with_conn(|conn| {
            for item_id in item_ids {
                let (weight, useful_count, useless_count): (f64, i64, i64) = conn
                    .query_row(
                        "SELECT weight, useful_count, useless_count FROM context_weights WHERE item_id = ?1",
                        params![item_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap_or((1.0, 0, 0));

                let (new_weight, new_useful, new_useless) = if useful {
                    (weight + 0.05, useful_count + 1, useless_count)
                } else {
                    (weight - 0.05, useful_count, useless_count + 1)
                };
                let new_weight = new_weight.clamp(0.5, 1.5);

                conn.execute(
                    r#"INSERT INTO context_weights (item_id, weight, useful_count, useless_count, last_used_at)
                       VALUES (?1,?2,?3,?4,?5)
                       ON CONFLICT(item_id) DO UPDATE SET
                         weight=excluded.weight,
                         useful_count=excluded.useful_count,
                         useless_count=excluded.useless_count,
                         last_used_at=excluded.last_used_at"#,
                    params![item_id, new_weight, new_useful, new_useless, now],
                )?;
                updated += 1;
            }
            Ok(())
        })?;
        Ok(updated)
    }

    fn load_context_weights(&self, ids: &[String]) -> Result<HashMap<String, f64>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.with_conn(|conn| {
            let mut map = HashMap::new();
            for id in ids {
                if let Ok(weight) = conn.query_row(
                    "SELECT weight FROM context_weights WHERE item_id = ?1",
                    params![id],
                    |r| r.get::<_, f64>(0),
                ) {
                    map.insert(id.clone(), weight);
                }
            }
            Ok(map)
        })
    }

    pub(crate) fn bm25_prefilter(&self, query: &str) -> Result<Bm25Prefilter> {
        let item_bm25 = self.bm25_search_items(query, BM25_ITEMS_TOP).unwrap_or_default();
        let fact_bm25 = self.bm25_search_facts(query, BM25_FACTS_TOP).unwrap_or_default();

        let mut bm25_map = HashMap::new();
        let mut bm25_max = 0.0f64;
        for (id, score) in item_bm25.iter().chain(fact_bm25.iter()) {
            bm25_max = bm25_max.max(score.abs());
            bm25_map.insert(id.clone(), *score);
        }

        Ok(Bm25Prefilter {
            item_ids: item_bm25.into_iter().map(|(id, _)| id).collect(),
            memory_ids: fact_bm25.into_iter().map(|(id, _)| id).collect(),
            bm25_map,
            bm25_max,
        })
    }

    pub(crate) fn score_items_with_bm25(
        &self,
        snapshot: &SearchIndexCache,
        query_embedding: &[f32],
        bm25: &Bm25Prefilter,
        repo_root: Option<&str>,
        tags: &[String],
        boost_agents: bool,
        bm25_only: bool,
        phase: Option<&str>,
        match_ctx: Option<&MatchContext<'_>>,
    ) -> Result<(Vec<ScoredItem>, usize, usize)> {
        let index_total = snapshot.indexed.len() + snapshot.memories.len();

        let mut candidate_ids = bm25.item_ids.clone();
        let mut candidates: Vec<&CachedRow> = Vec::new();
        for id in &bm25.item_ids {
            if let Some(&idx) = snapshot.indexed_by_id.get(id) {
                candidates.push(&snapshot.indexed[idx]);
            }
        }
        for id in &bm25.memory_ids {
            if let Some(&idx) = snapshot.memories_by_id.get(id) {
                candidates.push(&snapshot.memories[idx]);
            }
        }

        if candidates.len() < MIN_CANDIDATES {
            let mut recent = scoped_fallback_rows(snapshot, repo_root);
            for row in recent.drain(..) {
                if candidate_ids.insert(row.id.clone()) {
                    candidates.push(row);
                }
                if candidates.len() >= MIN_CANDIDATES {
                    break;
                }
            }
        }

        const MAX_EXTRA_MEMORIES: usize = 50;
        let mut included_ids: HashSet<String> = candidates.iter().map(|r| r.id.clone()).collect();
        let mut extra_memories: Vec<&CachedRow> = snapshot.memories.iter().collect();
        extra_memories.sort_by_key(|row| std::cmp::Reverse(row.updated_at));
        for row in extra_memories.into_iter().take(MAX_EXTRA_MEMORIES) {
            if included_ids.insert(row.id.clone()) {
                candidates.push(row);
            }
        }

        let candidate_count = candidates.len();
        let candidate_ids: Vec<String> = candidates.iter().map(|r| r.id.clone()).collect();
        let context_weights = self.load_context_weights(&candidate_ids)?;

        let cosine_sims: Vec<f64> = if bm25_only || query_embedding.is_empty() {
            vec![0.0; candidate_count]
        } else {
            let emb_refs: Vec<Option<&[f32]>> = candidates
                .iter()
                .map(|row| row.embedding.as_deref())
                .collect();
            batch_dot_products(query_embedding, &emb_refs)
        };

        let mut scored = Vec::with_capacity(candidate_count);
        for (row, cosine_sim) in candidates.into_iter().zip(cosine_sims) {
            let item_type = ItemType::parse(&row.item_type).unwrap_or(ItemType::Rule);

            let bm25_norm = bm25
                .bm25_map
                .get(&row.id)
                .map(|s| {
                    if bm25.bm25_max > 0.0 {
                        s.abs() / bm25.bm25_max
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);

            let mut score = if bm25_only {
                0.9 * bm25_norm
            } else {
                0.7 * cosine_sim + 0.2 * bm25_norm
            };

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

            if let Some(phase) = phase {
                score += phase_match_boost(phase, &row.topic, &row.text);
            }

            if let Some(weight) = context_weights.get(&row.id) {
                score *= weight;
            }

            let mut apply_when_matched = false;
            if item_type == ItemType::Memory {
                let meta = memory_fact_meta(snapshot, row);
                let source = meta.and_then(|m| m.source.as_deref());
                let confidence = meta.map(|m| m.confidence).unwrap_or(row.confidence);
                let apply_when = meta
                    .and_then(|m| m.apply_when.as_deref())
                    .or(row.apply_when.as_deref());
                let polarity = meta
                    .and_then(|m| m.polarity.as_deref())
                    .or(row.polarity.as_deref());

                if source == Some("user") || confidence >= 0.95 {
                    score += 0.08;
                }
                if let Some(ctx) = match_ctx {
                    let conditions = parse_apply_when(apply_when);
                    if matches_apply_when(&conditions, ctx) {
                        score += 0.15;
                        apply_when_matched = !conditions.is_empty();
                    } else if !conditions.is_empty() {
                        score *= 0.85;
                    }
                }

                scored.push(ScoredItem {
                    id: row.id.clone(),
                    item_type,
                    topic: row.topic.clone(),
                    text: row.text.clone(),
                    source_path: if row.source_path.is_empty() {
                        None
                    } else {
                        Some(row.source_path.clone())
                    },
                    scope: row.scope.clone(),
                    score,
                    polarity: polarity.map(str::to_string),
                    apply_when_matched,
                });
                continue;
            }

            scored.push(ScoredItem {
                id: row.id.clone(),
                item_type,
                topic: row.topic.clone(),
                text: row.text.clone(),
                source_path: if row.source_path.is_empty() {
                    None
                } else {
                    Some(row.source_path.clone())
                },
                scope: row.scope.clone(),
                score,
                polarity: row.polarity.clone(),
                apply_when_matched,
            });
        }

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok((scored, candidate_count, index_total))
    }

    pub fn score_items(
        &self,
        query: &str,
        query_embedding: &[f32],
        repo_root: Option<&str>,
        tags: &[String],
        boost_agents: bool,
        phase: Option<&str>,
        match_ctx: Option<&MatchContext<'_>>,
    ) -> Result<(Vec<ScoredItem>, usize, usize)> {
        let snapshot = self.search_cache_snapshot()?;
        let bm25 = self.bm25_prefilter(query)?;
        self.score_items_with_bm25(
            &snapshot,
            query_embedding,
            &bm25,
            repo_root,
            tags,
            boost_agents,
            false,
            phase,
            match_ctx,
        )
    }
}

fn memory_fact_meta<'a>(snapshot: &'a SearchIndexCache, row: &'a CachedRow) -> Option<&'a CachedRow> {
    snapshot.memories.iter().find(|m| {
        m.topic == row.topic
            && m.text == row.text
            && m.scope == row.scope
            && m.scope_key == row.scope_key
    })
}

fn scoped_fallback_rows<'a>(
    snapshot: &'a SearchIndexCache,
    repo_root: Option<&str>,
) -> Vec<&'a CachedRow> {
    let mut indices: Vec<usize> = Vec::new();
    let mut seen = HashSet::new();

    if let Some(root) = repo_root {
        if let Some(scoped) = snapshot.scoped_indices.get(root) {
            for &idx in scoped {
                if seen.insert(idx) {
                    indices.push(idx);
                }
            }
        }
    }
    for &idx in &snapshot.global_indices {
        if seen.insert(idx) {
            indices.push(idx);
        }
    }
    if indices.len() < FALLBACK_RECENT {
        for idx in 0..snapshot.indexed.len() {
            if seen.insert(idx) {
                indices.push(idx);
            }
            if indices.len() >= FALLBACK_RECENT {
                break;
            }
        }
    }

    let mut rows: Vec<&CachedRow> = indices.iter().map(|&i| &snapshot.indexed[i]).collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at));
    rows.truncate(FALLBACK_RECENT);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bm25_fast_path_requires_hits_and_strength() {
        let weak = Bm25Prefilter {
            item_ids: HashSet::from(["a".into()]),
            memory_ids: HashSet::new(),
            bm25_map: HashMap::from([("a".into(), -1.0)]),
            bm25_max: 1.0,
        };
        assert!(!weak.fast_path_eligible());

        let strong = Bm25Prefilter {
            item_ids: HashSet::from(["a".into()]),
            memory_ids: HashSet::new(),
            bm25_map: HashMap::from([("a".into(), -4.0)]),
            bm25_max: 4.0,
        };
        assert!(strong.fast_path_eligible());
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

#[derive(Debug, Clone)]
pub struct ActiveFactSnapshot {
    pub id: String,
    pub fact: String,
    pub updated_at: i64,
}

pub struct ConflictSnapshot {
    pub id: String,
    pub topic: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub loser_id: String,
    pub loser_fact: String,
    pub winner_id: String,
    pub restored: bool,
}

#[derive(Debug, Clone)]
pub struct RetrievalLogRow {
    pub id: String,
    pub query_hash: String,
    pub phase: String,
    pub items_json: String,
    pub tokens_used: usize,
    pub truncated: bool,
    pub cache_hit: bool,
    pub latency_ms: u64,
}

fn phase_match_boost(phase: &str, topic: &str, text: &str) -> f64 {
    if phase == "unknown" {
        return 0.0;
    }
    let hay = format!("{} {}", topic, text).to_lowercase();
    if hay.contains(phase) {
        return 0.12;
    }
    let keywords = match phase {
        "debugging" => ["debug", "fix", "error", "bug"],
        "planning" => ["plan", "design", "architect", "roadmap"],
        "implementing" => ["implement", "build", "add", "create"],
        "reviewing" => ["review", "pr", "audit", "lint"],
        _ => return 0.0,
    };
    if keywords.iter().any(|k| hay.contains(k)) {
        0.12
    } else {
        0.0
    }
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
