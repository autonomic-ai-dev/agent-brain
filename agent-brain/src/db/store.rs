use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::migrations;
use crate::embed::{dot_product, normalize_embedding};
use crate::types::{ItemType, ScoredItem};

const BM25_ITEMS_TOP: usize = 150;
const BM25_FACTS_TOP: usize = 40;
const MIN_CANDIDATES: usize = 50;
const FALLBACK_RECENT: usize = 80;
const QUERY_EMBEDDING_CACHE_MAX: usize = 500;

pub(crate) struct SearchIndexCache {
    version: u64,
    indexed: Vec<CachedRow>,
    indexed_by_id: HashMap<String, usize>,
    memories: Vec<CachedRow>,
    memories_by_id: HashMap<String, usize>,
}

pub(crate) struct Bm25Prefilter {
    item_ids: HashSet<String>,
    memory_ids: HashSet<String>,
    bm25_map: HashMap<String, f64>,
    bm25_max: f64,
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
        let memories_by_id: HashMap<String, usize> = memories
            .iter()
            .enumerate()
            .map(|(i, row)| (row.id.clone(), i))
            .collect();
        let cache = Arc::new(SearchIndexCache {
            version,
            indexed,
            indexed_by_id,
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
                r#"SELECT id, topic, fact, scope, scope_key, updated_at
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
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8,?9,?10)"#,
                params![id, topic, fact, scope, scope_key, source, confidence, now, expires, content_hash],
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
            let mut recent: Vec<&CachedRow> = snapshot.indexed.iter().collect();
            recent.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            for row in recent.into_iter().take(FALLBACK_RECENT) {
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
        extra_memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        for row in extra_memories.into_iter().take(MAX_EXTRA_MEMORIES) {
            if included_ids.insert(row.id.clone()) {
                candidates.push(row);
            }
        }

        let candidate_count = candidates.len();
        let mut scored = Vec::with_capacity(candidate_count);
        for row in candidates {
            let item_type = ItemType::parse(&row.item_type).unwrap_or(ItemType::Rule);
            let cosine_sim = if let Some(emb) = &row.embedding {
                dot_product(query_embedding, emb)
            } else {
                0.0
            };

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
        )
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
