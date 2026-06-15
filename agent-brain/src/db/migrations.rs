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

    Ok(())
}
