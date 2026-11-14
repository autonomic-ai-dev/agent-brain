use rusqlite::Connection;

const SCHEMA_VERSION: i64 = 29;

const VERIFY_COLUMNS: &[(&str, &str)] = &[
    ("changed_paths_json", "TEXT"),
    ("head_sha", "TEXT"),
    ("code_hash", "TEXT"),
    ("source", "TEXT NOT NULL DEFAULT 'cli'"),
    ("project_key", "TEXT"),
    ("tests_run", "INTEGER"),
];

pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS verify_outcomes (
            id TEXT PRIMARY KEY,
            route_log_id TEXT,
            exit_code INTEGER NOT NULL,
            cause_tag TEXT,
            verify_config_hash TEXT,
            scenario_dir_hash TEXT,
            cmd TEXT NOT NULL,
            device_id TEXT,
            recorded_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_verify_outcomes_at ON verify_outcomes(recorded_at);
        "#,
    )?;
    for (column, definition) in VERIFY_COLUMNS {
        if !column_exists(conn, "verify_outcomes", column)? {
            conn.execute(
                &format!("ALTER TABLE verify_outcomes ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_verify_outcomes_project ON verify_outcomes(project_key, recorded_at);",
    )?;
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [SCHEMA_VERSION],
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> i64 {
        conn.query_row("SELECT version FROM schema_version", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_fresh_database_gets_the_verify_schema_at_the_current_version() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        assert_eq!(version(&conn), SCHEMA_VERSION);
        for (column, _) in VERIFY_COLUMNS {
            assert!(column_exists(&conn, "verify_outcomes", column).unwrap(), "{column}");
        }
        assert!(!column_exists(&conn, "verify_outcomes", "nope").unwrap());
    }

    #[test]
    fn running_twice_keeps_one_version_row() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn an_older_verify_table_gets_the_missing_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE verify_outcomes (id TEXT PRIMARY KEY, route_log_id TEXT, exit_code INTEGER NOT NULL, cause_tag TEXT, verify_config_hash TEXT, scenario_dir_hash TEXT, cmd TEXT NOT NULL, device_id TEXT, recorded_at INTEGER NOT NULL);
             CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (12);",
        )
        .unwrap();
        run(&conn).unwrap();
        for (column, _) in VERIFY_COLUMNS {
            assert!(column_exists(&conn, "verify_outcomes", column).unwrap(), "{column}");
        }
        assert_eq!(version(&conn), SCHEMA_VERSION);
    }
}
