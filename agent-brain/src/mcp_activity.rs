use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tracks in-flight MCP tool calls so auto-restart can wait for a quiet window.
pub struct McpActivity {
    in_flight: AtomicU32,
    last_complete_ms: AtomicU64,
    last_route_ms: AtomicU64,
}

pub struct McpRequestGuard<'a> {
    activity: &'a McpActivity,
}

impl Default for McpActivity {
    fn default() -> Self {
        let now = now_ms();
        Self {
            in_flight: AtomicU32::new(0),
            last_complete_ms: AtomicU64::new(now),
            last_route_ms: AtomicU64::new(now),
        }
    }
}

impl McpActivity {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_request(&self) -> McpRequestGuard<'_> {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        McpRequestGuard { activity: self }
    }

    pub fn idle_for_secs(&self, secs: u64) -> bool {
        if self.in_flight.load(Ordering::Acquire) != 0 {
            return false;
        }
        let idle_ms = secs.saturating_mul(1000);
        now_ms().saturating_sub(self.last_complete_ms.load(Ordering::Relaxed)) >= idle_ms
    }

    /// Record that route_task completed. Called after the route response is built.
    pub fn record_route(&self) {
        self.last_route_ms
            .store(self.last_complete_ms.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    /// Returns true if any non-route MCP tool completed since the last route_task.
    pub fn tools_used_since_last_route(&self) -> bool {
        self.last_complete_ms.load(Ordering::Relaxed)
            > self.last_route_ms.load(Ordering::Relaxed)
    }
}

impl Drop for McpRequestGuard<'_> {
    fn drop(&mut self) {
        self.activity.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.activity
            .last_complete_ms
            .store(now_ms(), Ordering::Relaxed);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn idle_after_request_completes() {
        let activity = McpActivity::new();
        assert!(activity.idle_for_secs(0));
        {
            let _guard = activity.begin_request();
            assert!(!activity.idle_for_secs(0));
        }
        assert!(activity.idle_for_secs(0));
    }

    #[test]
    fn idle_waits_for_quiet_period() {
        let activity = McpActivity::new();
        {
            let _guard = activity.begin_request();
        }
        assert!(!activity.idle_for_secs(1));
        thread::sleep(Duration::from_millis(20));
        assert!(activity.idle_for_secs(0));
    }
}
