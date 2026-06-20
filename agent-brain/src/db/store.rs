#![allow(clippy::too_many_arguments)] // row-oriented SQLite APIs take many optional columns

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::ann::AnnIndex;
use crate::db::migrations;
use crate::embed::{batch_dot_products, normalize_embedding};
use crate::intelligence::{matches_apply_when, parse_apply_when, MatchContext};
use crate::types::{ItemType, ScoredItem};

const BM25_ITEMS_TOP: usize = 150;
const BM25_FACTS_TOP: usize = 40;
const MIN_CANDIDATES: usize = 15;
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
    ann: Option<AnnIndex>,
}

pub(crate) struct Bm25Prefilter {
    item_ids: HashSet<String>,
    memory_ids: HashSet<String>,
    bm25_map: HashMap<String, f64>,
    bm25_max: f64,
}

const SUPERVISOR_FAST_PATH_MIN_STRENGTH: f64 = 1.8;

impl Bm25Prefilter {
    pub(crate) fn fast_path_eligible(&self, query: &str) -> bool {
        if self.item_ids.is_empty() {
            return false;
        }
        if self.bm25_max >= BM25_FAST_PATH_MIN_STRENGTH {
            return true;
        }
        crate::retrieval::supervisor_query_strength(query) >= 0.25
            && self.bm25_max >= SUPERVISOR_FAST_PATH_MIN_STRENGTH
    }
}

#[derive(Clone)]
pub(crate) struct CachedRow {
    pub(crate) id: String,
    pub(crate) item_type: String,
    pub(crate) topic: String,
    pub(crate) text: String,
    pub(crate) source_path: String,
    pub(crate) scope: String,
    pub(crate) scope_key: Option<String>,
    pub(crate) embedding: Option<Vec<f32>>,
    updated_at: i64,
    polarity: Option<String>,
    source: Option<String>,
    confidence: f64,
    apply_when: Option<String>,
}

#[cfg(test)]
impl CachedRow {
    pub(crate) fn test_with_embedding(id: &str, emb: Vec<f32>) -> Self {
        Self::test_with_embedding_scoped(id, emb, "global", None)
    }

    pub(crate) fn test_with_embedding_scoped(
        id: &str,
        emb: Vec<f32>,
        scope: &str,
        scope_key: Option<&str>,
    ) -> Self {
        Self {
            id: id.into(),
            item_type: "skill".into(),
            topic: id.into(),
            text: String::new(),
            source_path: String::new(),
            scope: scope.into(),
            scope_key: scope_key.map(str::to_string),
            embedding: Some(emb),
            updated_at: 0,
            polarity: None,
            source: None,
            confidence: 0.9,
            apply_when: None,
        }
    }
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
        let ann = AnnIndex::from_rows(&indexed);
        let cache = Arc::new(SearchIndexCache {
            version,
            indexed,
            indexed_by_id,
            global_indices,
            scoped_indices,
            memories,
            memories_by_id,
            ann,
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
            let mut stmt = conn.prepare(&format!(
                r#"SELECT id, topic, fact, scope, scope_key, updated_at, polarity, source, confidence, apply_when
                   FROM facts WHERE superseded_by IS NULL
                     AND (expires_at IS NULL OR expires_at > ?1)
                     AND {}"#,
                crate::temporal::active_fact_sql("?1")
            ))?;
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
        let strict = crate::retrieval::fts_query_strict(query);
        let hits = self.bm25_search_items_fts(&strict, limit)?;
        if hits.len() >= 5 {
            return Ok(hits);
        }
        let loose = crate::retrieval::fts_query_loose(query);
        if loose == strict {
            return Ok(hits);
        }
        let mut merged = hits;
        let loose_hits = self.bm25_search_items_fts(&loose, limit)?;
        for (id, score) in loose_hits {
            if merged.iter().any(|(existing, _)| existing == &id) {
                continue;
            }
            merged.push((id, score));
            if merged.len() >= limit {
                break;
            }
        }
        Ok(merged)
    }

    fn bm25_search_items_fts(&self, fts_query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        if fts_query == "\"\"" {
            return Ok(Vec::new());
        }
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT i.id, bm25(items_fts) AS score
                   FROM items_fts
                   JOIN indexed_items i ON i.rowid = items_fts.rowid
                   WHERE items_fts MATCH ?1
                   ORDER BY score
                   LIMIT ?2"#,
            )?;
            let rows = stmt
                .query_map(params![fts_query, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn bm25_search_facts(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        let strict = crate::retrieval::fts_query_strict(query);
        let hits = self.bm25_search_facts_fts(&strict, limit)?;
        if hits.len() >= 3 {
            return Ok(hits);
        }
        let loose = crate::retrieval::fts_query_loose(query);
        if loose == strict {
            return Ok(hits);
        }
        let mut merged = hits;
        for (id, score) in self.bm25_search_facts_fts(&loose, limit)? {
            if merged.iter().any(|(existing, _)| existing == &id) {
                continue;
            }
            merged.push((id, score));
            if merged.len() >= limit {
                break;
            }
        }
        Ok(merged)
    }

    fn bm25_search_facts_fts(&self, fts_query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        if fts_query == "\"\"" {
            return Ok(Vec::new());
        }
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
            let rows = stmt
                .query_map(params![fts_query, now, limit as i64], |row| {
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

    /// Flush WAL pages into the main db file (for committing a portable fixture).
    pub fn checkpoint_wal(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    pub fn count_indexed_items(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM indexed_items", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn count_indexed_items_matching(&self, sql: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn count_active_facts(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = chrono::Utc::now().timestamp_millis();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM facts WHERE superseded_by IS NULL AND (expires_at IS NULL OR expires_at > ?1)",
            params![now],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Facts inserted since `since_ms` (includes superseded rows — each store counts as a commit).
    pub fn count_facts_created_since(&self, since_ms: i64) -> Result<usize> {
        self.with_conn(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM facts WHERE created_at >= ?1",
                params![since_ms],
                |r| r.get(0),
            )?;
            Ok(n.max(0) as usize)
        })
    }

    pub fn count_indexed_by_type(&self) -> Result<Vec<(String, usize)>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT item_type, COUNT(*) FROM indexed_items GROUP BY item_type")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
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

    pub fn replace_upstream_tools(
        &self,
        tools: &[crate::upstream::IndexedUpstreamTool],
    ) -> Result<()> {
        let json = serde_json::to_string(tools)?;
        self.set_meta("upstream_tools_index", &json)
    }

    pub fn list_upstream_tools(&self) -> Result<Vec<crate::upstream::IndexedUpstreamTool>> {
        match self.get_meta("upstream_tools_index")? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
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
        temporal: Option<&FactTemporal>,
    ) -> Result<StoreFactResult> {
        let polarity = if polarity == "negative" {
            "negative"
        } else {
            "positive"
        };
        let now = chrono::Utc::now().timestamp_millis();
        let valid_from = temporal.and_then(|t| t.valid_from).unwrap_or(now);
        let invalid_at = temporal.and_then(|t| t.invalid_at);
        if let Some(until) = invalid_at {
            if until <= valid_from {
                anyhow::bail!("invalid_at must be after valid_from");
            }
        }
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

        let prior: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT id FROM facts WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND superseded_by IS NULL AND {}",
                        crate::temporal::active_fact_sql("?4")
                    ),
                    params![topic, scope, scope_key, now],
                    |r| r.get(0),
                )
                .optional()?)
        })?;

        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO facts (id, topic, fact, scope, scope_key, source, confidence, created_at, updated_at, expires_at, content_hash, polarity, apply_when, valid_from, invalid_at)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8,?9,?10,?11,?12,?13,?14)"#,
                params![
                    id,
                    topic,
                    fact,
                    scope,
                    scope_key,
                    source,
                    confidence,
                    now,
                    expires,
                    content_hash,
                    polarity,
                    apply_when,
                    valid_from,
                    invalid_at
                ],
            )?;
            if let Some(old_id) = &prior {
                crate::temporal::link_fact_evolution(conn, &id, old_id, "evolved_from", now)?;
            }
            Ok(())
        })?;

        let blob: Vec<u8> = normalize_embedding(embedding.to_vec())
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        if crate::temporal::is_fact_active(now, valid_from, invalid_at) {
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
        }

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

    /// Topics with multiple active agent/user facts — candidates for observation synthesis.
    pub fn list_recurring_memory_topics(
        &self,
        min_count: usize,
        since_ms: i64,
    ) -> Result<Vec<RecurringMemoryTopic>> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT topic, scope, scope_key, COUNT(*) as cnt
                 FROM facts
                 WHERE superseded_by IS NULL
                   AND source NOT IN ('observation')
                   AND topic NOT LIKE 'obs/%'
                   AND updated_at >= ?1
                   AND {}
                 GROUP BY topic, scope, scope_key
                 HAVING cnt >= ?2",
                crate::temporal::active_fact_sql("?3")
            );
            let mut stmt = conn.prepare(&sql)?;
            let groups: Vec<(String, String, Option<String>, usize)> = stmt
                .query_map(params![since_ms, min_count as i64, now], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get::<_, i64>(3)? as usize,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            let mut out = Vec::new();
            for (topic, scope, scope_key, fact_count) in groups {
                let detail_sql = format!(
                    "SELECT fact, polarity, apply_when FROM facts
                     WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'')
                       AND superseded_by IS NULL AND source NOT IN ('observation')
                       AND {}
                     ORDER BY updated_at DESC",
                    crate::temporal::active_fact_sql("?4")
                );
                let mut detail = conn.prepare(&detail_sql)?;
                let rows: Vec<(String, String, Option<String>)> = detail
                    .query_map(params![topic, scope, scope_key, now], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let latest_fact = rows[0].0.clone();
                let negative_count = rows.iter().filter(|(_, pol, _)| pol == "negative").count();
                let mut tag_set = std::collections::HashSet::new();
                for (_, _, apply_when) in &rows {
                    for tag in parse_apply_when(apply_when.as_deref()) {
                        tag_set.insert(tag);
                    }
                }
                let mut apply_when_tags: Vec<String> = tag_set.into_iter().collect();
                apply_when_tags.sort();
                out.push(RecurringMemoryTopic {
                    topic,
                    scope,
                    scope_key,
                    fact_count,
                    latest_fact,
                    negative_count,
                    apply_when_tags,
                });
            }
            Ok(out)
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

    pub fn list_secret_refs(&self) -> Result<Vec<crate::secrets::SecretRef>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT name, used_by FROM secret_refs ORDER BY name ASC")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::secrets::SecretRef {
                        name: row.get(0)?,
                        used_by: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn upsert_secret_ref(&self, name: &str, used_by: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO secret_refs (name, used_by, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(name) DO UPDATE SET used_by = excluded.used_by",
                params![name, used_by, now],
            )?;
            Ok(())
        })
    }

    pub fn merge_secret_refs(&self, refs: &[crate::secrets::SecretRef]) -> Result<usize> {
        let mut added = 0usize;
        for reference in refs {
            let exists = self.with_conn(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT 1 FROM secret_refs WHERE name = ?1 LIMIT 1",
                        params![reference.name],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            })?;
            if !exists {
                self.upsert_secret_ref(&reference.name, &reference.used_by)?;
                added += 1;
            }
        }
        Ok(added)
    }

    pub fn cloud_last_push_ms(&self) -> Result<Option<i64>> {
        Ok(self
            .get_meta("cloud_last_push_ms")?
            .and_then(|v| v.parse().ok()))
    }

    pub fn cloud_last_pull_ms(&self) -> Result<Option<i64>> {
        Ok(self
            .get_meta("cloud_last_pull_ms")?
            .and_then(|v| v.parse().ok()))
    }

    pub fn set_cloud_last_push(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.set_meta("cloud_last_push_ms", &now.to_string())
    }

    pub fn set_cloud_last_pull(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.set_meta("cloud_last_pull_ms", &now.to_string())
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

    pub fn prune_expired_facts(&self) -> Result<crate::temporal::PruneReport> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| crate::temporal::prune_expired(conn, now))
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
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT id, fact, updated_at FROM facts
                     WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'')
                       AND superseded_by IS NULL
                       AND {}
                     ORDER BY updated_at DESC
                     LIMIT 1",
                    crate::temporal::active_fact_sql("?4")
                ),
                params![topic, scope, scope_key, now],
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

    /// Test helper: backdate a fact for GC eligibility checks.
    pub fn set_fact_updated_at_for_test(&self, id: &str, updated_at: i64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE facts SET updated_at = ?1 WHERE id = ?2",
                params![updated_at, id],
            )?;
            Ok(())
        })
    }

    /// Test helper: backdate context feedback for low-signal GC eligibility.
    pub fn set_context_last_used_for_test(&self, item_id: &str, last_used_at: i64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE context_weights SET last_used_at = ?1 WHERE item_id = ?2",
                params![last_used_at, item_id],
            )?;
            Ok(())
        })
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
        index_total: Option<usize>,
        saved_pct: Option<u8>,
        must_apply_count: Option<usize>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO retrieval_log (id, timestamp, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms, index_total, saved_pct, must_apply_count)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"#,
                params![
                    id,
                    now,
                    query_hash,
                    phase,
                    items_json,
                    tokens_used as i64,
                    truncated as i64,
                    cache_hit as i64,
                    latency_ms as i64,
                    index_total.map(|n| n as i64),
                    saved_pct.map(|n| n as i64),
                    must_apply_count.map(|n| n as i64),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_retrieval_log(&self, id: &str) -> Result<Option<RetrievalLogRow>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms, index_total, saved_pct, must_apply_count FROM retrieval_log WHERE id = ?1",
                params![id],
                map_retrieval_log_row,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn latest_retrieval_log(&self) -> Result<Option<RetrievalLogRow>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms, index_total, saved_pct, must_apply_count FROM retrieval_log ORDER BY timestamp DESC LIMIT 1",
                [],
                map_retrieval_log_row,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn list_retrieval_logs(&self, limit: usize) -> Result<Vec<RetrievalLogRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, query_hash, phase, items_returned, tokens_used, truncated, cache_hit, latency_ms, index_total, saved_pct, must_apply_count FROM retrieval_log ORDER BY timestamp DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], map_retrieval_log_row)?
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
                "SELECT id, topic, scope, scope_key, loser_id, loser_fact, winner_id, winner_fact, restored FROM conflict_log WHERE id = ?1",
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
                        winner_fact: row.get(7)?,
                        restored: row.get::<_, i64>(8)? != 0,
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

    pub fn delete_fact_by_topic_text(
        &self,
        topic: &str,
        scope: &str,
        scope_key: Option<&str>,
        fact: &str,
    ) -> Result<usize> {
        self.with_conn(|conn| {
            let deleted = conn.execute(
                "DELETE FROM facts WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'') AND fact = ?4",
                params![topic, scope, scope_key, fact],
            )?;
            Ok(deleted)
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

    pub fn insert_skill_staging(
        &self,
        id: &str,
        fact_id: &str,
        topic: &str,
        skill_name: &str,
        draft_path: &str,
        target_path: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO skill_staging (id, fact_id, topic, skill_name, draft_path, target_path, status, created_at)
                   VALUES (?1,?2,?3,?4,?5,?6,'pending',?7)"#,
                params![id, fact_id, topic, skill_name, draft_path, target_path, now],
            )?;
            Ok(())
        })
    }

    pub fn list_skill_staging(&self, status: Option<&str>) -> Result<Vec<SkillStagingRow>> {
        self.with_conn(|conn| {
            let mut rows = Vec::new();
            if let Some(status) = status {
                let mut stmt = conn.prepare(
                    "SELECT id, fact_id, topic, skill_name, draft_path, target_path, status, created_at, resolved_at
                     FROM skill_staging WHERE status = ?1 ORDER BY created_at DESC",
                )?;
                let mapped = stmt.query_map(params![status], map_skill_staging_row)?;
                for row in mapped {
                    rows.push(row?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, fact_id, topic, skill_name, draft_path, target_path, status, created_at, resolved_at
                     FROM skill_staging ORDER BY created_at DESC",
                )?;
                let mapped = stmt.query_map([], map_skill_staging_row)?;
                for row in mapped {
                    rows.push(row?);
                }
            }
            Ok(rows)
        })
    }

    pub fn get_skill_staging(&self, id: &str) -> Result<Option<SkillStagingRow>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, fact_id, topic, skill_name, draft_path, target_path, status, created_at, resolved_at
                 FROM skill_staging WHERE id = ?1",
                params![id],
                map_skill_staging_row,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn resolve_skill_staging(&self, id: &str, status: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE skill_staging SET status = ?1, resolved_at = ?2 WHERE id = ?3",
                params![status, now, id],
            )?;
            Ok(())
        })
    }

    pub fn list_gc_candidates(
        &self,
        now_ms: i64,
        stale_ms: i64,
        very_stale_ms: i64,
    ) -> Result<Vec<GcCandidate>> {
        let stale_cutoff = now_ms - stale_ms;
        let very_stale_cutoff = now_ms - very_stale_ms;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT f.id, f.topic, f.fact, f.scope, f.scope_key, f.source, f.confidence, f.polarity, f.apply_when,
                          CASE
                            WHEN cw.weight IS NOT NULL AND cw.weight < 0.6 AND cw.useless_count > cw.useful_count
                                 AND IFNULL(cw.last_used_at, 0) < ?1 THEN 'low_signal'
                            WHEN cw.item_id IS NULL AND f.source IN ('session_digest', 'legacy', 'legacy_cursor')
                                 AND f.updated_at < ?2 THEN 'stale_session_digest'
                            ELSE 'unknown'
                          END AS gc_kind
                   FROM facts f
                   LEFT JOIN context_weights cw ON cw.item_id = f.id
                   WHERE f.superseded_by IS NULL
                     AND (
                       (cw.weight IS NOT NULL AND cw.weight < 0.6 AND cw.useless_count > cw.useful_count
                        AND IFNULL(cw.last_used_at, 0) < ?1)
                       OR (cw.item_id IS NULL AND f.source IN ('session_digest', 'legacy', 'legacy_cursor')
                           AND f.updated_at < ?2)
                     )"#,
            )?;
            let rows = stmt
                .query_map(params![stale_cutoff, very_stale_cutoff], |row| {
                    Ok(GcCandidate {
                        id: row.get(0)?,
                        topic: row.get(1)?,
                        fact: row.get(2)?,
                        scope: row.get(3)?,
                        scope_key: row.get(4)?,
                        source: row.get(5)?,
                        confidence: row.get(6)?,
                        polarity: row.get(7)?,
                        apply_when: row.get(8)?,
                        gc_kind: row.get(9)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn archive_fact(&self, candidate: &GcCandidate, reason: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let archive_id = Uuid::new_v4().to_string();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO facts_archive (id, original_id, topic, fact, scope, scope_key, source, confidence, polarity, apply_when, archived_at, archive_reason)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"#,
                params![
                    archive_id,
                    candidate.id,
                    candidate.topic,
                    candidate.fact,
                    candidate.scope,
                    candidate.scope_key,
                    candidate.source,
                    candidate.confidence,
                    candidate.polarity,
                    candidate.apply_when,
                    now,
                    reason
                ],
            )?;
            conn.execute("DELETE FROM facts WHERE id = ?1", params![candidate.id])?;
            conn.execute(
                "DELETE FROM indexed_items WHERE item_type = 'memory' AND topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'')",
                params![candidate.topic, candidate.scope, candidate.scope_key],
            )?;
            conn.execute(
                "DELETE FROM context_weights WHERE item_id = ?1",
                params![candidate.id],
            )?;
            Ok(())
        })?;
        self.invalidate_search_cache();
        Ok(())
    }

    pub fn insert_tool_log(
        &self,
        id: &str,
        tool_name: &str,
        path: Option<&str>,
        detail: Option<&str>,
        tokens_used: usize,
        tokens_saved: Option<usize>,
        savings_pct: Option<f64>,
        must_apply_active: bool,
        phase: Option<&str>,
        route_log_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO tool_log (id, timestamp, tool_name, path, detail, tokens_used, tokens_saved, savings_pct, must_apply_active, phase, route_log_id)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
                params![
                    id,
                    now,
                    tool_name,
                    path,
                    detail,
                    tokens_used as i64,
                    tokens_saved.map(|n| n as i64),
                    savings_pct.map(|n| n.round() as i64),
                    must_apply_active as i64,
                    phase,
                    route_log_id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_pending_tool_traces(&self, limit: usize) -> Result<Vec<ToolTraceRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT t.id, t.tool_name, t.path, t.detail
                   FROM tool_log t
                   LEFT JOIN trace_extract_log e ON e.tool_log_id = t.id
                   WHERE e.tool_log_id IS NULL
                   ORDER BY t.timestamp ASC
                   LIMIT ?1"#,
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(ToolTraceRow {
                        id: row.get(0)?,
                        tool_name: row.get(1)?,
                        path: row.get(2)?,
                        detail: row.get(3)?,
                        scope_key: None,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
    }

    pub fn mark_trace_extracted(&self, tool_log_id: &str, fact_id: Option<&str>) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO trace_extract_log (tool_log_id, fact_id, extracted_at) VALUES (?1, ?2, ?3)",
                params![tool_log_id, fact_id, now],
            )?;
            Ok(())
        })
    }

    pub fn list_active_fact_ids_for_topic(
        &self,
        topic: &str,
        scope: &str,
        scope_key: Option<&str>,
    ) -> Result<Vec<String>> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT id FROM facts
                 WHERE topic = ?1 AND scope = ?2 AND IFNULL(scope_key,'') = IFNULL(?3,'')
                   AND superseded_by IS NULL AND {}
                 ORDER BY updated_at DESC",
                crate::temporal::active_fact_sql("?4")
            );
            let mut stmt = conn.prepare(&sql)?;
            let ids = stmt
                .query_map(params![topic, scope, scope_key, now], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(ids)
        })
    }

    pub fn insert_fact_lineage(
        &self,
        fact_id: &str,
        source_type: &str,
        source_id: &str,
        relation: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO fact_lineage (id, fact_id, source_type, source_id, relation, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    Uuid::new_v4().to_string(),
                    fact_id,
                    source_type,
                    source_id,
                    relation,
                    now
                ],
            )?;
            Ok(())
        })
    }

    pub fn count_fact_lineage(&self, fact_id: &str) -> Result<usize> {
        self.with_conn(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM fact_lineage WHERE fact_id = ?1",
                params![fact_id],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
    }

    pub fn insert_workflow_trajectory(
        &self,
        workflow_id: &str,
        node_id: &str,
        outcome: &str,
        route_log_id: Option<&str>,
        task_kind: Option<&str>,
        notes: Option<&str>,
    ) -> Result<TrajectoryRecord> {
        if route_log_id.is_some_and(|id| !id.trim().is_empty()) {
            let id = route_log_id.unwrap().trim();
            if self.get_retrieval_log(id)?.is_none() {
                anyhow::bail!("route_log_id not found: {id}");
            }
        }
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO workflow_trajectory
                   (id, workflow_id, node_id, outcome, route_log_id, task_kind, notes, recorded_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    id,
                    workflow_id,
                    node_id,
                    outcome,
                    route_log_id,
                    task_kind,
                    notes,
                    now
                ],
            )?;
            Ok(())
        })?;
        Ok(TrajectoryRecord {
            id,
            workflow_id: workflow_id.to_string(),
            node_id: node_id.to_string(),
            outcome: outcome.to_string(),
            route_log_id: route_log_id.map(str::to_string),
            task_kind: task_kind.map(str::to_string),
            notes: notes.map(str::to_string),
            recorded_at: now,
        })
    }

    pub fn query_trajectories(&self) -> Result<Vec<TrajectoryRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, workflow_id, node_id, outcome, route_log_id, task_kind, notes, recorded_at
                 FROM workflow_trajectory ORDER BY recorded_at ASC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(TrajectoryRecord {
                        id: row.get(0)?,
                        workflow_id: row.get(1)?,
                        node_id: row.get(2)?,
                        outcome: row.get(3)?,
                        route_log_id: row.get(4)?,
                        task_kind: row.get(5)?,
                        notes: row.get(6)?,
                        recorded_at: row.get(7)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn tool_log_stats_since(&self, since_ms: i64) -> Result<(usize, u64, f64, usize)> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT tool_name, tokens_saved, savings_pct FROM tool_log WHERE timestamp >= ?1",
            )?;
            let rows = stmt
                .query_map(params![since_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut tool_calls = 0usize;
            let mut tokens_saved = 0u64;
            let mut savings_pcts = Vec::new();
            let mut inefficient_reads = 0usize;

            for (name, saved, pct) in rows {
                tool_calls += 1;
                if name == "cursor_read" {
                    inefficient_reads += 1;
                }
                if let Some(s) = saved {
                    tokens_saved += s.max(0) as u64;
                }
                if let Some(p) = pct {
                    savings_pcts.push(p.max(0) as u8);
                }
            }

            let avg_savings = if savings_pcts.is_empty() {
                0.0
            } else {
                savings_pcts.iter().map(|p| *p as f64).sum::<f64>() / savings_pcts.len() as f64
            };

            Ok((tool_calls, tokens_saved, avg_savings, inefficient_reads))
        })
    }

    pub fn retrieval_stats_since(&self, since_ms: i64) -> Result<RetrievalStats> {
        let mut stats = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT phase, cache_hit, latency_ms, tokens_used, index_total, saved_pct, must_apply_count FROM retrieval_log WHERE timestamp >= ?1",
            )?;
            let rows = stmt
                .query_map(params![since_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, u64>(2)?,
                        row.get::<_, i64>(3)? as usize,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut route_calls = 0usize;
            let mut upstream_calls = 0usize;
            let mut cache_hits = 0usize;
            let mut latencies = Vec::new();
            let mut phase_counts: HashMap<String, usize> = HashMap::new();
            let mut savings_pcts = Vec::new();
            let mut total_saved_tokens = 0u64;
            let mut routed_token_sum = 0u64;
            let mut routed_token_rows = 0usize;
            let mut routes_with_constraints = 0usize;
            let mut total_must_apply = 0usize;

            for (phase, cache_hit, latency, tokens_used, index_total, saved_pct, must_apply_count) in rows {
                if phase == "upstream_call" {
                    upstream_calls += 1;
                } else {
                    route_calls += 1;
                    routed_token_sum += tokens_used as u64;
                    routed_token_rows += 1;
                    if let Some(pct) = saved_pct {
                        savings_pcts.push(pct as u8);
                        if let Some(index_n) = index_total {
                            let index_n = index_n.max(0) as usize;
                            let naive = index_n
                                .saturating_mul(crate::route_briefing::NAIVE_TOKENS_PER_INDEXED_ITEM);
                            let saved = naive.saturating_sub(tokens_used);
                            total_saved_tokens += saved as u64;
                        }
                    }
                    if let Some(n) = must_apply_count {
                        let n = n.max(0) as usize;
                        if n > 0 {
                            routes_with_constraints += 1;
                            total_must_apply += n;
                        }
                    }
                }
                if cache_hit {
                    cache_hits += 1;
                }
                latencies.push(latency);
                *phase_counts.entry(phase).or_default() += 1;
            }

            let total = route_calls + upstream_calls;
            let cache_hit_rate = if total == 0 {
                0.0
            } else {
                cache_hits as f64 / total as f64
            };
            let avg_latency_ms = if latencies.is_empty() {
                0.0
            } else {
                latencies.iter().sum::<u64>() as f64 / latencies.len() as f64
            };
            latencies.sort_unstable();
            let p95_latency_ms = if latencies.is_empty() {
                0
            } else {
                let idx = ((latencies.len() as f64 * 0.95).ceil() as usize)
                    .saturating_sub(1)
                    .min(latencies.len() - 1);
                latencies[idx]
            };
            let avg_saved_pct = if savings_pcts.is_empty() {
                0.0
            } else {
                savings_pcts.iter().map(|p| *p as f64).sum::<f64>() / savings_pcts.len() as f64
            };
            let avg_routed_tokens = if routed_token_rows == 0 {
                0.0
            } else {
                routed_token_sum as f64 / routed_token_rows as f64
            };

            let mut phases: Vec<(String, usize)> = phase_counts.into_iter().collect();
            phases.sort_by(|a, b| b.1.cmp(&a.1));

            Ok(RetrievalStats {
                route_calls,
                upstream_calls,
                cache_hit_rate,
                avg_latency_ms,
                p95_latency_ms,
                phases,
                routes_with_savings: savings_pcts.len(),
                avg_saved_pct,
                total_saved_tokens,
                avg_routed_tokens,
                routes_with_constraints,
                total_must_apply,
                tool_calls: 0,
                tool_tokens_saved: 0,
                tool_avg_savings_pct: 0.0,
                inefficient_read_events: 0,
            })
        })?;
        let (tool_calls, tool_tokens_saved, tool_avg_savings_pct, inefficient_read_events) =
            self.tool_log_stats_since(since_ms)?;
        stats.tool_calls = tool_calls;
        stats.tool_tokens_saved = tool_tokens_saved;
        stats.tool_avg_savings_pct = tool_avg_savings_pct;
        stats.inefficient_read_events = inefficient_read_events;
        Ok(stats)
    }

    pub fn context_feedback_summary(&self, lowest_n: usize) -> Result<ContextFeedbackSummary> {
        self.with_conn(|conn| {
            let (items_tracked, total_useful, total_useless): (i64, i64, i64) = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(useful_count),0), COALESCE(SUM(useless_count),0) FROM context_weights",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;

            let mut stmt = conn.prepare(
                "SELECT item_id, weight, useful_count, useless_count FROM context_weights ORDER BY weight ASC LIMIT ?1",
            )?;
            let lowest_weight = stmt
                .query_map(params![lowest_n as i64], |row| {
                    Ok(WeightSnapshot {
                        item_id: row.get(0)?,
                        weight: row.get(1)?,
                        useful_count: row.get(2)?,
                        useless_count: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(ContextFeedbackSummary {
                items_tracked: items_tracked as usize,
                total_useful,
                total_useless,
                lowest_weight,
            })
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
        let item_bm25 = self
            .bm25_search_items(query, BM25_ITEMS_TOP)
            .unwrap_or_default();
        let fact_bm25 = self
            .bm25_search_facts(query, BM25_FACTS_TOP)
            .unwrap_or_default();

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
        query: &str,
        query_embedding: &[f32],
        bm25: &Bm25Prefilter,
        repo_root: Option<&str>,
        tags: &[String],
        boost_agents: bool,
        bm25_only: bool,
        phase: Option<&str>,
        match_ctx: Option<&MatchContext<'_>>,
        ann_settings: crate::ann::AnnSettings,
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

        if ann_settings.enabled && !bm25_only && !query_embedding.is_empty() {
            if let Some(ann) = snapshot.ann.as_ref() {
                if ann.is_active(ann_settings.min_index) {
                    let filter = repo_root.map(|root| crate::ann::AnnFilter {
                        repo_root: Some(root),
                    });
                    for (id, _) in
                        ann.top_k_filtered(query_embedding, ann_settings.top_k, filter.as_ref())
                    {
                        if candidate_ids.insert(id.clone()) {
                            if let Some(&idx) = snapshot.indexed_by_id.get(&id) {
                                candidates.push(&snapshot.indexed[idx]);
                            }
                        }
                    }
                }
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

        const MAX_EXTRA_MEMORIES: usize = 12;
        let mut included_ids: HashSet<String> = candidates.iter().map(|r| r.id.clone()).collect();

        const MAX_PINNED_NEGATIVE: usize = 5;
        let mut pinned_negative = 0usize;
        for row in &snapshot.memories {
            if pinned_negative >= MAX_PINNED_NEGATIVE {
                break;
            }
            let polarity = memory_fact_meta(snapshot, row)
                .and_then(|m| m.polarity.as_deref())
                .or(row.polarity.as_deref());
            if polarity == Some("negative") && included_ids.insert(row.id.clone()) {
                candidates.push(row);
                pinned_negative += 1;
            }
        }

        let mut extra_memories: Vec<&CachedRow> = snapshot
            .memories
            .iter()
            .filter(|row| {
                !crate::workspace::is_low_signal_memory(&row.topic, row.source.as_deref())
            })
            .collect();
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
            let lexical = crate::retrieval::lexical_overlap_score(query, &row.topic, &row.text);
            let entity = crate::retrieval::entity_overlap_score(query, &row.topic, &row.text);

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
                0.60 * bm25_norm + 0.25 * lexical + 0.15 * entity
            } else {
                0.50 * cosine_sim + 0.22 * bm25_norm + 0.18 * lexical + 0.10 * entity
            };

            if matches!(item_type, ItemType::Skill | ItemType::Agent) && lexical >= 0.2 {
                score += 0.10 * lexical;
            }

            if item_type == ItemType::Skill {
                const SUPERVISOR_SKILLS: &[&str] = &["token-efficient-ops", "execution-supervisor"];
                if SUPERVISOR_SKILLS.contains(&row.topic.as_str())
                    && crate::retrieval::supervisor_query_strength(query) >= 0.2
                {
                    score += 0.18;
                }
            }

            if bm25_norm == 0.0 && lexical < 0.12 {
                score *= 0.5;
            }

            if let Some(root) = repo_root {
                if row.scope_key.as_deref() == Some(root) {
                    score += 0.1;
                }
            }

            for tag in tags {
                if row.topic.to_lowercase().contains(tag) || row.text.to_lowercase().contains(tag) {
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
                if source == Some("observation") {
                    score += 0.10;
                }
                if polarity == Some("negative") {
                    score += 0.12;
                }
                score += memory_source_score_adjustment(source, &row.topic);
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

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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
            query,
            query_embedding,
            &bm25,
            repo_root,
            tags,
            boost_agents,
            false,
            phase,
            match_ctx,
            crate::ann::AnnSettings::default(),
        )
    }
}

fn memory_fact_meta<'a>(
    snapshot: &'a SearchIndexCache,
    row: &'a CachedRow,
) -> Option<&'a CachedRow> {
    snapshot.memories.iter().find(|m| {
        m.topic == row.topic
            && m.text == row.text
            && m.scope == row.scope
            && m.scope_key == row.scope_key
    })
}

fn memory_source_score_adjustment(source: Option<&str>, topic: &str) -> f64 {
    if crate::workspace::is_low_signal_memory(topic, source) {
        return -0.30;
    }
    0.0
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
        assert!(!weak.fast_path_eligible("unrelated query"));
        assert!(!weak.fast_path_eligible("grep before cat large log token efficient"));

        let strong = Bm25Prefilter {
            item_ids: HashSet::from(["a".into()]),
            memory_ids: HashSet::new(),
            bm25_map: HashMap::from([("a".into(), -4.0)]),
            bm25_max: 4.0,
        };
        assert!(strong.fast_path_eligible("any query"));

        let supervisor_medium = Bm25Prefilter {
            item_ids: HashSet::from(["a".into()]),
            memory_ids: HashSet::new(),
            bm25_map: HashMap::from([("a".into(), -2.0)]),
            bm25_max: 2.0,
        };
        assert!(supervisor_medium.fast_path_eligible("grep before cat large log token efficient"));
    }

    #[test]
    fn low_signal_memory_penalized() {
        assert!(memory_source_score_adjustment(Some("session_digest"), "session-digest-x") < 0.0);
        assert!(memory_source_score_adjustment(None, "legacy-cursor-de") < 0.0);
        assert_eq!(memory_source_score_adjustment(Some("user"), "routing"), 0.0);
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

#[derive(Debug, Clone, Copy, Default)]
pub struct FactTemporal {
    pub valid_from: Option<i64>,
    pub invalid_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RecurringMemoryTopic {
    pub topic: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub fact_count: usize,
    pub latest_fact: String,
    pub negative_count: usize,
    pub apply_when_tags: Vec<String>,
}

pub struct StoreFactResult {
    pub id: String,
    pub stored: bool,
    pub deduplicated: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrajectoryRecord {
    pub id: String,
    pub workflow_id: String,
    pub node_id: String,
    pub outcome: String,
    pub route_log_id: Option<String>,
    pub task_kind: Option<String>,
    pub notes: Option<String>,
    pub recorded_at: i64,
}

#[derive(Debug, Clone)]
pub struct ActiveFactSnapshot {
    pub id: String,
    pub fact: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillStagingRow {
    pub id: String,
    pub fact_id: String,
    pub topic: String,
    pub skill_name: String,
    pub draft_path: String,
    pub target_path: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct GcCandidate {
    pub id: String,
    pub topic: String,
    pub fact: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub source: Option<String>,
    pub confidence: f64,
    pub polarity: Option<String>,
    pub apply_when: Option<String>,
    pub gc_kind: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievalStats {
    pub route_calls: usize,
    pub upstream_calls: usize,
    pub cache_hit_rate: f64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: u64,
    pub phases: Vec<(String, usize)>,
    pub routes_with_savings: usize,
    pub avg_saved_pct: f64,
    pub total_saved_tokens: u64,
    pub avg_routed_tokens: f64,
    pub routes_with_constraints: usize,
    pub total_must_apply: usize,
    pub tool_calls: usize,
    pub tool_tokens_saved: u64,
    pub tool_avg_savings_pct: f64,
    pub inefficient_read_events: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolEventRecord {
    pub timestamp: i64,
    pub tool_name: String,
    pub path: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    pub tokens_used: usize,
    pub tokens_saved: Option<usize>,
    pub savings_pct: Option<f64>,
    pub must_apply_active: bool,
    pub phase: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolTraceRow {
    pub id: String,
    pub tool_name: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub scope_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextFeedbackSummary {
    pub items_tracked: usize,
    pub total_useful: i64,
    pub total_useless: i64,
    pub lowest_weight: Vec<WeightSnapshot>,
}

#[derive(Debug, Clone)]
pub struct WeightSnapshot {
    pub item_id: String,
    pub weight: f64,
    pub useful_count: i64,
    pub useless_count: i64,
}

pub struct ConflictSnapshot {
    pub id: String,
    pub topic: String,
    pub scope: String,
    pub scope_key: Option<String>,
    pub loser_id: String,
    pub loser_fact: String,
    pub winner_id: String,
    pub winner_fact: String,
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
    pub index_total: Option<usize>,
    pub saved_pct: Option<u8>,
    pub must_apply_count: Option<usize>,
}

fn map_retrieval_log_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RetrievalLogRow> {
    Ok(RetrievalLogRow {
        id: row.get(0)?,
        query_hash: row.get(1)?,
        phase: row.get(2)?,
        items_json: row.get(3)?,
        tokens_used: row.get::<_, i64>(4)? as usize,
        truncated: row.get::<_, i64>(5)? != 0,
        cache_hit: row.get::<_, i64>(6)? != 0,
        latency_ms: row.get::<_, i64>(7)? as u64,
        index_total: row.get::<_, Option<i64>>(8)?.map(|n| n.max(0) as usize),
        saved_pct: row.get::<_, Option<i64>>(9)?.map(|n| n.clamp(0, 100) as u8),
        must_apply_count: row.get::<_, Option<i64>>(10)?.map(|n| n.max(0) as usize),
    })
}

fn phase_match_boost(phase: &str, topic: &str, text: &str) -> f64 {
    if phase == "unknown" {
        return 0.0;
    }
    let hay = format!("{} {}", topic, text).to_lowercase();
    if hay.contains(phase) {
        return 0.12;
    }
    let keywords: &[&str] = match phase {
        "debugging" => &["debug", "fix", "error", "bug", "fail", "crash", "issue"],
        "planning" => &["plan", "design", "architect", "roadmap", "spec", "version"],
        "implementing" => &[
            "implement",
            "build",
            "add",
            "create",
            "release",
            "sync",
            "mcp",
        ],
        "reviewing" => &["review", "pr", "audit", "lint", "checklist"],
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
    crate::retrieval::fts_query_loose(query)
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

fn map_skill_staging_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillStagingRow> {
    Ok(SkillStagingRow {
        id: row.get(0)?,
        fact_id: row.get(1)?,
        topic: row.get(2)?,
        skill_name: row.get(3)?,
        draft_path: row.get(4)?,
        target_path: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        resolved_at: row.get(8)?,
    })
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
    patterns.iter().any(|p| {
        regex::Regex::new(p)
            .map(|re| re.is_match(text))
            .unwrap_or(false)
    }) || text.contains(".env")
        || text.contains(".pem")
}
