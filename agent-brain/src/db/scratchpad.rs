//! Cross-agent scratchpad — always available (not gated behind extras/graphify).

use anyhow::Result;
use rusqlite::params;
use uuid::Uuid;

use crate::code_nav::ScratchpadEntry;
use crate::db::store::BrainStore;

impl BrainStore {
    pub fn write_scratchpad(
        &self,
        repo_root: &str,
        agent_id: Option<&str>,
        content: &str,
        context_type: &str,
        tags: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO code_scratchpad (id, repo_root, agent_id, content, context_type, tags, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, repo_root, agent_id, content, context_type, tags, now],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    pub fn read_scratchpad(
        &self,
        repo_root: &str,
        limit: usize,
        context_type: Option<&str>,
    ) -> Result<Vec<ScratchpadEntry>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT id, repo_root, agent_id, content, context_type, tags, created_at, updated_at
                 FROM code_scratchpad
                 WHERE repo_root = ?1",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(repo_root.to_string())];
            if let Some(ct) = context_type {
                sql.push_str(" AND context_type = ?2");
                params.push(Box::new(ct.to_string()));
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            let limit_idx = params.len() + 1;
            params.push(Box::new(limit as i64));
            sql.push_str(&limit_idx.to_string());

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |r| {
                Ok(ScratchpadEntry {
                    id: r.get(0)?,
                    repo_root: r.get(1)?,
                    agent_id: r.get(2)?,
                    content: r.get(3)?,
                    context_type: r.get(4)?,
                    tags: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            })?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    }
}
