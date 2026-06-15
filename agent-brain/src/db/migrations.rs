use rusqlite::Connection;

pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA busy_timeout=5000;
        PRAGMA synchronous=NORMAL;

        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS indexed_items (
            id TEXT PRIMARY KEY,
            item_type TEXT NOT NULL,
            topic TEXT NOT NULL,
            text TEXT NOT NULL,
            source_path TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            scope_key TEXT,
            content_hash TEXT NOT NULL,
            embedding BLOB,
            updated_at INTEGER NOT NULL,
            UNIQUE(source_path, content_hash)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
            topic, text, content='indexed_items', content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS items_ai AFTER INSERT ON indexed_items BEGIN
            INSERT INTO items_fts(rowid, topic, text) VALUES (new.rowid, new.topic, new.text);
        END;
        CREATE TRIGGER IF NOT EXISTS items_ad AFTER DELETE ON indexed_items BEGIN
            INSERT INTO items_fts(items_fts, rowid, topic, text) VALUES('delete', old.rowid, old.topic, old.text);
        END;
        CREATE TRIGGER IF NOT EXISTS items_au AFTER UPDATE ON indexed_items BEGIN
            INSERT INTO items_fts(items_fts, rowid, topic, text) VALUES('delete', old.rowid, old.topic, old.text);
            INSERT INTO items_fts(rowid, topic, text) VALUES (new.rowid, new.topic, new.text);
        END;

        CREATE TABLE IF NOT EXISTS facts (
            id TEXT PRIMARY KEY,
            topic TEXT NOT NULL,
            fact TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_key TEXT,
            source TEXT NOT NULL DEFAULT 'agent',
            confidence REAL DEFAULT 0.9,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_used_at INTEGER,
            expires_at INTEGER,
            content_hash TEXT NOT NULL,
            superseded_by TEXT,
            UNIQUE(content_hash, scope, scope_key)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(
            topic, fact, content='facts', content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
            INSERT INTO facts_fts(rowid, topic, fact) VALUES (new.rowid, new.topic, new.fact);
        END;
        CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
            INSERT INTO facts_fts(facts_fts, rowid, topic, fact) VALUES('delete', old.rowid, old.topic, old.fact);
        END;
        CREATE TRIGGER IF NOT EXISTS facts_au AFTER UPDATE ON facts BEGIN
            INSERT INTO facts_fts(facts_fts, rowid, topic, fact) VALUES('delete', old.rowid, old.topic, old.fact);
            INSERT INTO facts_fts(rowid, topic, fact) VALUES (new.rowid, new.topic, new.fact);
        END;

        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS query_embeddings (
            query_hash TEXT PRIMARY KEY,
            embedding BLOB NOT NULL,
            updated_at INTEGER NOT NULL
        );
        "#,
    )?;

    let version: i64 = conn
        .query_row(
            "SELECT COALESCE((SELECT version FROM schema_version LIMIT 1), 0)",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if version == 0 {
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])?;
        conn.execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('index_version', '1')",
            [],
        )?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 2 {
        migrate_v2(conn)?;
        conn.execute("UPDATE schema_version SET version = 2", [])?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 3 {
        migrate_v3(conn)?;
        conn.execute("UPDATE schema_version SET version = 3", [])?;
    }

    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_v2(conn: &Connection) -> rusqlite::Result<()> {
    if !column_exists(conn, "facts", "polarity")? {
        conn.execute(
            "ALTER TABLE facts ADD COLUMN polarity TEXT NOT NULL DEFAULT 'positive'",
            [],
        )?;
    }

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS conflict_log (
            id TEXT PRIMARY KEY,
            timestamp INTEGER NOT NULL,
            sync_source TEXT NOT NULL DEFAULT 'local',
            topic TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_key TEXT,
            loser_id TEXT NOT NULL,
            loser_fact TEXT NOT NULL,
            winner_id TEXT NOT NULL,
            winner_fact TEXT NOT NULL,
            resolution TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS retrieval_log (
            id TEXT PRIMARY KEY,
            timestamp INTEGER NOT NULL,
            query_hash TEXT NOT NULL,
            phase TEXT NOT NULL,
            items_returned TEXT NOT NULL,
            tokens_used INTEGER NOT NULL,
            truncated INTEGER NOT NULL,
            cache_hit INTEGER NOT NULL,
            latency_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS context_weights (
            item_id TEXT PRIMARY KEY,
            weight REAL NOT NULL DEFAULT 1.0,
            useful_count INTEGER NOT NULL DEFAULT 0,
            useless_count INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER
        );
        "#,
    )?;
    Ok(())
}

fn migrate_v3(conn: &Connection) -> rusqlite::Result<()> {
    if !column_exists(conn, "facts", "apply_when")? {
        conn.execute("ALTER TABLE facts ADD COLUMN apply_when TEXT", [])?;
    }
    Ok(())
}
