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

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 4 {
        migrate_v4(conn)?;
        conn.execute("UPDATE schema_version SET version = 4", [])?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 5 {
        migrate_v5(conn)?;
        conn.execute("UPDATE schema_version SET version = 5", [])?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 6 {
        migrate_v6(conn)?;
        conn.execute("UPDATE schema_version SET version = 6", [])?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 7 {
        migrate_v7(conn)?;
        conn.execute("UPDATE schema_version SET version = 7", [])?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 8 {
        migrate_v8(conn)?;
        conn.execute("UPDATE schema_version SET version = 8", [])?;
    }

    let version: i64 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version < 9 {
        migrate_v9(conn)?;
        conn.execute("UPDATE schema_version SET version = 9", [])?;
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

fn migrate_v4(conn: &Connection) -> rusqlite::Result<()> {
    if !column_exists(conn, "conflict_log", "restored")? {
        conn.execute(
            "ALTER TABLE conflict_log ADD COLUMN restored INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v5(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS secret_refs (
            name TEXT PRIMARY KEY,
            used_by TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        "#,
    )?;
    Ok(())
}

fn migrate_v6(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS skill_staging (
            id TEXT PRIMARY KEY,
            fact_id TEXT NOT NULL,
            topic TEXT NOT NULL,
            skill_name TEXT NOT NULL,
            draft_path TEXT NOT NULL,
            target_path TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS facts_archive (
            id TEXT PRIMARY KEY,
            original_id TEXT NOT NULL,
            topic TEXT NOT NULL,
            fact TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_key TEXT,
            source TEXT NOT NULL,
            confidence REAL,
            polarity TEXT,
            apply_when TEXT,
            archived_at INTEGER NOT NULL,
            archive_reason TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

fn migrate_v7(conn: &Connection) -> rusqlite::Result<()> {
    if !column_exists(conn, "retrieval_log", "index_total")? {
        conn.execute("ALTER TABLE retrieval_log ADD COLUMN index_total INTEGER", [])?;
    }
    if !column_exists(conn, "retrieval_log", "saved_pct")? {
        conn.execute("ALTER TABLE retrieval_log ADD COLUMN saved_pct INTEGER", [])?;
    }
    Ok(())
}

fn migrate_v8(conn: &Connection) -> rusqlite::Result<()> {
    if !column_exists(conn, "retrieval_log", "must_apply_count")? {
        conn.execute(
            "ALTER TABLE retrieval_log ADD COLUMN must_apply_count INTEGER",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v9(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tool_log (
            id TEXT PRIMARY KEY,
            timestamp INTEGER NOT NULL,
            tool_name TEXT NOT NULL,
            path TEXT,
            tokens_used INTEGER NOT NULL,
            tokens_saved INTEGER,
            savings_pct INTEGER,
            must_apply_active INTEGER NOT NULL DEFAULT 0,
            phase TEXT,
            route_log_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_tool_log_timestamp ON tool_log(timestamp);
        "#,
    )?;
    Ok(())
}
