use std::fs;
use std::path::Path;

use anyhow::Result;
use rusqlite::params;
use sha2::{Digest, Sha256};

use crate::db::store::{looks_like_secret, BrainStore};
use crate::types::LessonRec;

const DAY_MS: i64 = 24 * 3600 * 1000;
const LESSON_TTL_MS: i64 = 60 * DAY_MS;
const EXCERPT_CHARS: usize = 300;
const MAX_PATHS: usize = 20;
const MAX_ROUTE_LESSONS: usize = 3;
const FAILURE_MARKERS: [&str; 4] = ["error", "FAILED", "panicked", "assert"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lesson {
    pub id: String,
    pub project_key: String,
    pub paths: Vec<String>,
    pub cmd: String,
    pub excerpt: String,
    pub created_at: i64,
}

fn lesson_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Lesson> {
    let paths_json: String = row.get(2)?;
    Ok(Lesson {
        id: row.get(0)?,
        project_key: row.get(1)?,
        paths: serde_json::from_str(&paths_json).unwrap_or_default(),
        cmd: row.get(3)?,
        excerpt: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub fn excerpt_of(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines
        .iter()
        .find(|line| FAILURE_MARKERS.iter().any(|marker| line.contains(marker)))
        .or(lines.last())
        .map(|line| line.chars().take(EXCERPT_CHARS).collect())
        .unwrap_or_default()
}

pub fn touches(paths: &[String], touched: &[String]) -> bool {
    paths.iter().any(|lesson_path| {
        touched
            .iter()
            .any(|path| path == lesson_path || path.starts_with(&format!("{lesson_path}/")))
    })
}

fn lesson_id(project_key: &str, cmd: &str, paths: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{project_key}\n{cmd}\n{}", paths.join("\n")));
    format!("{:x}", hasher.finalize())
}

pub fn current_code_paths(root: &Path) -> Vec<String> {
    crate::stop_gate::changed_paths(root)
        .into_iter()
        .filter(|path| crate::verify::is_code_path(path))
        .collect()
}

pub fn learn_from_fix(
    store: &BrainStore,
    root: &Path,
    project_key: &str,
    cmd: &str,
    failed_run_id: &str,
    failed_artifact: &Path,
) -> Result<()> {
    let Ok(output) = fs::read_to_string(failed_artifact) else {
        return Ok(());
    };
    let excerpt = excerpt_of(&output);
    if excerpt.is_empty() || looks_like_secret(&excerpt) {
        return Ok(());
    }
    let fixing = current_code_paths(root);
    let mut paths: Vec<String> = store
        .verify_outcome_changed_paths(failed_run_id)?
        .into_iter()
        .filter(|path| fixing.contains(path))
        .collect();
    paths.truncate(MAX_PATHS);
    if paths.is_empty() {
        return Ok(());
    }
    store.insert_lesson(&Lesson {
        id: lesson_id(project_key, cmd, &paths),
        project_key: project_key.to_string(),
        paths,
        cmd: cmd.to_string(),
        excerpt,
        created_at: chrono::Utc::now().timestamp_millis(),
    })
}

pub fn prune_expired_lessons(store: &BrainStore, now: i64) -> Result<u64> {
    store.delete_lessons_created_before(now - LESSON_TTL_MS)
}

pub fn route_lessons(store: &BrainStore, root: &Path, open_files: &[String]) -> Vec<LessonRec> {
    let Some(project_key) = crate::project_key::project_key_if_known(root) else {
        return Vec::new();
    };
    let now = chrono::Utc::now().timestamp_millis();
    let mut touched: Vec<String> = open_files
        .iter()
        .map(|file| {
            Path::new(file)
                .strip_prefix(root)
                .unwrap_or(Path::new(file))
                .display()
                .to_string()
        })
        .collect();
    touched.extend(crate::stop_gate::changed_paths(root));
    store
        .lessons_for_project(&project_key, now)
        .unwrap_or_default()
        .into_iter()
        .filter(|lesson| touches(&lesson.paths, &touched))
        .take(MAX_ROUTE_LESSONS)
        .map(|lesson| LessonRec {
            paths: lesson.paths,
            cmd: lesson.cmd,
            excerpt: lesson.excerpt,
            age_days: u32::try_from((now - lesson.created_at) / DAY_MS).unwrap_or(u32::MAX),
        })
        .collect()
}

impl BrainStore {
    pub fn set_verify_outcome_changed_paths(&self, id: &str, paths: &[String]) -> Result<()> {
        let json = serde_json::to_string(paths)?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE verify_outcomes SET changed_paths_json = ?1 WHERE id = ?2",
                params![json, id],
            )?;
            Ok(())
        })
    }

    pub fn verify_outcome_changed_paths(&self, id: &str) -> Result<Vec<String>> {
        self.with_conn(|conn| {
            let json: Option<String> = conn
                .query_row(
                    "SELECT changed_paths_json FROM verify_outcomes WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .unwrap_or(None);
            Ok(json
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default())
        })
    }

    pub fn insert_lesson(&self, lesson: &Lesson) -> Result<()> {
        let paths = serde_json::to_string(&lesson.paths)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO lessons (id, project_key, paths_json, cmd, excerpt, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    lesson.id,
                    lesson.project_key,
                    paths,
                    lesson.cmd,
                    lesson.excerpt,
                    lesson.created_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_lessons_created_before(&self, cutoff: i64) -> Result<u64> {
        self.with_conn(|conn| {
            let removed =
                conn.execute("DELETE FROM lessons WHERE created_at < ?1", params![cutoff])?;
            Ok(removed as u64)
        })
    }

    pub fn lessons_all(&self, now: i64) -> Result<Vec<Lesson>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_key, paths_json, cmd, excerpt, created_at FROM lessons
                 WHERE created_at >= ?1 ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map(params![now - LESSON_TTL_MS], lesson_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn lessons_for_project(&self, project_key: &str, now: i64) -> Result<Vec<Lesson>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_key, paths_json, cmd, excerpt, created_at FROM lessons
                 WHERE project_key = ?1 AND created_at >= ?2 ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map(params![project_key, now - LESSON_TTL_MS], lesson_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn lesson(paths: &[&str], created_at: i64) -> Lesson {
        Lesson {
            id: lesson_id(
                "p",
                "cargo test",
                &paths.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            ),
            project_key: "p".into(),
            paths: paths.iter().map(|p| p.to_string()).collect(),
            cmd: "cargo test".into(),
            excerpt: "assertion failed".into(),
            created_at,
        }
    }

    #[test]
    fn the_excerpt_is_the_first_failure_line() {
        let output = "compiling\nrunning 3 tests\nthread 'x' panicked at src/a.rs:4\nall done";
        assert_eq!(excerpt_of(output), "thread 'x' panicked at src/a.rs:4");
    }

    #[test]
    fn a_lesson_touches_its_own_paths_and_their_directories_only() {
        let paths = vec!["src/gate".to_string()];
        assert!(touches(&paths, &["src/gate/a.rs".to_string()]));
        assert!(touches(&paths, &["src/gate".to_string()]));
        assert!(!touches(&paths, &["src/gatekeeper.rs".to_string()]));
    }

    #[test]
    fn lessons_are_listed_for_their_project_within_the_ttl() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        store.insert_lesson(&lesson(&["src/a.rs"], now)).unwrap();
        store
            .insert_lesson(&lesson(&["src/old.rs"], now - LESSON_TTL_MS - DAY_MS))
            .unwrap();
        let listed = store.lessons_for_project("p", now).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].paths, ["src/a.rs"]);
        assert!(store.lessons_for_project("other", now).unwrap().is_empty());
    }

    #[test]
    fn lessons_past_the_ttl_are_pruned_and_recent_ones_are_kept() {
        let dir = tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("t.db")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        store.insert_lesson(&lesson(&["src/a.rs"], now)).unwrap();
        store
            .insert_lesson(&lesson(&["src/old.rs"], now - LESSON_TTL_MS - DAY_MS))
            .unwrap();
        assert_eq!(prune_expired_lessons(&store, now).unwrap(), 1);
        let listed = store.lessons_for_project("p", now).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].paths, ["src/a.rs"]);
        assert_eq!(prune_expired_lessons(&store, now).unwrap(), 0);
    }

    #[test]
    fn a_fix_after_a_failure_stores_one_lesson_for_the_changed_code() {
        let repo = tempdir().unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        fs::create_dir_all(repo.path().join("src")).unwrap();
        fs::write(repo.path().join("src/a.rs"), "fn a() {}").unwrap();
        let artifact = repo.path().join("run.log");
        fs::write(&artifact, "running\nassertion failed: left == right\n").unwrap();
        let db = tempdir().unwrap();
        let store = BrainStore::open(&db.path().join("t.db")).unwrap();
        let failed_id = store
            .insert_verify_outcome(
                None,
                1,
                None,
                None,
                None,
                "cargo test",
                "dev",
                "cli",
                Some("p"),
            )
            .unwrap();
        store
            .set_verify_outcome_changed_paths(
                &failed_id,
                &["src/a.rs".to_string(), "src/b.rs".to_string()],
            )
            .unwrap();

        learn_from_fix(
            &store,
            repo.path(),
            "p",
            "cargo test",
            &failed_id,
            &artifact,
        )
        .unwrap();
        learn_from_fix(
            &store,
            repo.path(),
            "p",
            "cargo test",
            &failed_id,
            &artifact,
        )
        .unwrap();

        let listed = store
            .lessons_for_project("p", chrono::Utc::now().timestamp_millis())
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].excerpt, "assertion failed: left == right");
        assert_eq!(listed[0].paths, ["src/a.rs"]);
    }
}
