use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::serve_lock::lock_identity;

pub const DEFAULT_IDLE_SECS: u64 = 3600;
const CHECK_EVERY: Duration = Duration::from_secs(30);

pub struct DaemonActivity {
    sessions: AtomicUsize,
    last_activity_secs: AtomicU64,
}

impl DaemonActivity {
    pub fn new() -> Self {
        Self {
            sessions: AtomicUsize::new(0),
            last_activity_secs: AtomicU64::new(now_secs()),
        }
    }

    pub fn begin_session(self: &Arc<Self>) -> SessionGuard {
        self.sessions.fetch_add(1, Ordering::SeqCst);
        self.touch();
        SessionGuard {
            activity: Arc::clone(self),
        }
    }

    fn touch(&self) {
        self.last_activity_secs.store(now_secs(), Ordering::SeqCst);
    }

    fn sessions(&self) -> usize {
        self.sessions.load(Ordering::SeqCst)
    }

    fn idle_secs(&self) -> u64 {
        now_secs().saturating_sub(self.last_activity_secs.load(Ordering::SeqCst))
    }
}

pub struct SessionGuard {
    activity: Arc<DaemonActivity>,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.activity.sessions.fetch_sub(1, Ordering::SeqCst);
        self.activity.touch();
    }
}

pub fn should_exit_idle(sessions: usize, idle_secs: u64, idle_limit_secs: u64) -> bool {
    sessions == 0 && idle_secs >= idle_limit_secs
}

pub fn lifeline_broken(
    home: &std::path::Path,
    lock_id: Option<(u64, u64)>,
    socket_path: &std::path::Path,
) -> bool {
    let lock_gone = lock_id.is_some_and(|id| lock_identity(home) != Some(id));
    lock_gone || !socket_path.exists()
}

pub fn idle_limit_from_env() -> u64 {
    std::env::var("AGENT_BRAIN_DAEMON_IDLE_SECS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_IDLE_SECS)
}

pub fn spawn(
    home: PathBuf,
    socket_path: PathBuf,
    activity: Arc<DaemonActivity>,
    idle_limit_secs: u64,
) {
    let lock_id = lock_identity(&home);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CHECK_EVERY).await;
            if lifeline_broken(&home, lock_id, &socket_path) {
                tracing::info!("daemon: serve lock or socket is gone, exiting");
                std::process::exit(0);
            }
            if should_exit_idle(activity.sessions(), activity.idle_secs(), idle_limit_secs) {
                tracing::info!(
                    idle_limit_secs,
                    "daemon: no clients for the idle limit, exiting"
                );
                std::process::exit(0);
            }
        }
    });
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn idle_exit_needs_no_sessions_and_the_full_limit() {
        assert!(should_exit_idle(0, 3600, 3600));
        assert!(!should_exit_idle(0, 3599, 3600));
        assert!(!should_exit_idle(1, 99_999, 3600));
    }

    #[test]
    fn sessions_are_counted_and_released_on_drop() {
        let activity = Arc::new(DaemonActivity::new());
        let first = activity.begin_session();
        let second = activity.begin_session();
        assert_eq!(activity.sessions(), 2);
        drop(first);
        assert_eq!(activity.sessions(), 1);
        drop(second);
        assert_eq!(activity.sessions(), 0);
    }

    #[test]
    fn lifeline_holds_while_the_lock_and_socket_are_intact() {
        let home = tempdir().unwrap();
        let socket = home.path().join("serve.sock");
        std::fs::write(&socket, b"").unwrap();
        std::fs::write(home.path().join("serve.lock"), b"pid=1").unwrap();
        let lock_id = lock_identity(home.path());
        assert!(!lifeline_broken(home.path(), lock_id, &socket));
    }

    #[test]
    fn lifeline_breaks_when_the_socket_is_removed() {
        let home = tempdir().unwrap();
        let socket = home.path().join("serve.sock");
        std::fs::write(&socket, b"").unwrap();
        std::fs::write(home.path().join("serve.lock"), b"pid=1").unwrap();
        let lock_id = lock_identity(home.path());
        std::fs::remove_file(&socket).unwrap();
        assert!(lifeline_broken(home.path(), lock_id, &socket));
    }

    #[test]
    fn lifeline_breaks_when_the_home_is_deleted() {
        let home = tempdir().unwrap();
        let path = home.path().to_path_buf();
        let socket = path.join("serve.sock");
        std::fs::write(&socket, b"").unwrap();
        std::fs::write(path.join("serve.lock"), b"pid=1").unwrap();
        let lock_id = lock_identity(&path);
        drop(home);
        assert!(lifeline_broken(&path, lock_id, &socket));
    }

    #[test]
    fn lifeline_breaks_when_the_lock_file_is_replaced() {
        let home = tempdir().unwrap();
        let socket = home.path().join("serve.sock");
        std::fs::write(&socket, b"").unwrap();
        std::fs::write(home.path().join("serve.lock"), b"pid=1").unwrap();
        let lock_id = lock_identity(home.path());
        let original = std::fs::File::open(home.path().join("serve.lock")).unwrap();
        std::fs::remove_file(home.path().join("serve.lock")).unwrap();
        std::fs::write(home.path().join("serve.lock"), b"pid=2").unwrap();
        assert!(lifeline_broken(home.path(), lock_id, &socket));
        drop(original);
    }

    #[test]
    fn idle_limit_defaults_to_one_hour() {
        assert_eq!(DEFAULT_IDLE_SECS, 3600);
    }
}
