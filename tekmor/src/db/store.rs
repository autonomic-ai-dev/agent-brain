#![allow(clippy::too_many_arguments)] // row-oriented SQLite APIs take many optional columns


use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::migrations;

pub struct BrainStore {
    conn: Arc<Mutex<Connection>>,
}

impl BrainStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path).context("open brain.db")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .context("open brain.db: if an earlier build encrypted it (TEKMOR_ENCRYPT_DB), it cannot be read now; restore an unencrypted copy")?;
        migrations::run(&conn).context("run migrations")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),

        })
    }

    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let guard = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        f(&guard)
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

    pub fn ensure_device_id(&self) -> Result<String> {
        if let Some(id) = self.get_meta("device_id")? {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        self.set_meta("device_id", &id)?;
        Ok(id)
    }

    pub fn insert_verify_outcome(
        &self,
        route_log_id: Option<&str>,
        exit_code: i32,
        cause_tag: Option<&str>,
        verify_config_hash: Option<&str>,
        scenario_dir_hash: Option<&str>,
        cmd: &str,
        device_id: &str,
        source: &str,
        project_key: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO verify_outcomes
                   (id, route_log_id, exit_code, cause_tag, verify_config_hash, scenario_dir_hash, cmd, device_id, recorded_at, source, project_key)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"#,
                params![
                    id,
                    route_log_id,
                    exit_code,
                    cause_tag,
                    verify_config_hash,
                    scenario_dir_hash,
                    cmd,
                    device_id,
                    now,
                    source,
                    project_key
                ],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn set_verify_outcome_tests_run(&self, id: &str, tests_run: u64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE verify_outcomes SET tests_run = ?1 WHERE id = ?2",
                params![tests_run as i64, id],
            )?;
            Ok(())
        })
    }

    pub fn set_verify_outcome_code_hash(&self, id: &str, code_hash: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE verify_outcomes SET code_hash = ?1 WHERE id = ?2",
                params![code_hash, id],
            )?;
            Ok(())
        })
    }

    pub fn set_verify_outcome_head_sha(&self, id: &str, head_sha: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE verify_outcomes SET head_sha = ?1 WHERE id = ?2",
                params![head_sha, id],
            )?;
            Ok(())
        })
    }

    pub fn verify_outcome_tests_run(&self, id: &str) -> Result<Option<i64>> {
        self.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT tests_run FROM verify_outcomes WHERE id = ?1",
                params![id],
                |r| r.get::<_, Option<i64>>(0),
            )?)
        })
    }

    pub fn list_verify_outcomes(&self, limit: usize) -> Result<Vec<VerifyOutcomeRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT id, route_log_id, exit_code, cause_tag, verify_config_hash, scenario_dir_hash, cmd, device_id, recorded_at, source, code_hash, head_sha
                   FROM verify_outcomes ORDER BY recorded_at DESC LIMIT ?1"#,
            )?;
            let rows = stmt
                .query_map(params![limit as i64], map_verify_outcome_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn list_verify_outcomes_for_project(
        &self,
        project_key: &str,
        limit: usize,
    ) -> Result<Vec<VerifyOutcomeRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT id, route_log_id, exit_code, cause_tag, verify_config_hash, scenario_dir_hash, cmd, device_id, recorded_at, source, code_hash, head_sha
                   FROM verify_outcomes WHERE project_key = ?1 ORDER BY recorded_at DESC LIMIT ?2"#,
            )?;
            let rows = stmt
                .query_map(params![project_key, limit as i64], map_verify_outcome_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

}

pub struct VerifyOutcomeRow {
    pub id: String,
    pub route_log_id: Option<String>,
    pub exit_code: i32,
    pub cause_tag: Option<String>,
    pub verify_config_hash: Option<String>,
    pub scenario_dir_hash: Option<String>,
    pub cmd: String,
    pub device_id: Option<String>,
    pub recorded_at: i64,
    pub source: Option<String>,
    pub code_hash: Option<String>,
    pub head_sha: Option<String>,
}

impl From<VerifyOutcomeRow> for tekmor_core::evidence::EvidenceRecord {
    fn from(row: VerifyOutcomeRow) -> Self {
        Self {
            id: row.id,
            cmd: row.cmd,
            exit_code: row.exit_code,
            recorded_at: row.recorded_at,
            source: row.source,
            code_hash: row.code_hash,
            head_sha: row.head_sha,
            verify_config_hash: row.verify_config_hash,
        }
    }
}

fn map_verify_outcome_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerifyOutcomeRow> {
    Ok(VerifyOutcomeRow {
        id: row.get(0)?,
        route_log_id: row.get(1)?,
        exit_code: row.get(2)?,
        cause_tag: row.get(3)?,
        verify_config_hash: row.get(4)?,
        scenario_dir_hash: row.get(5)?,
        cmd: row.get(6)?,
        device_id: row.get(7)?,
        recorded_at: row.get(8)?,
        source: row.get(9)?,
        code_hash: row.get(10)?,
        head_sha: row.get(11)?,
    })
}
