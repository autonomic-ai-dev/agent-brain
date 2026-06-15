use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default)]
pub struct RouteTiming {
    pub embed_us: u64,
    pub score_us: u64,
    pub build_us: u64,
    pub total_us: u64,
    pub cache_hit: bool,
    pub embed_cache_hit: bool,
    pub cache_warm: bool,
    pub candidates: usize,
    pub index_total: usize,
}

pub struct RouteLatencyStats {
    inner: Mutex<VecDeque<u64>>,
    cap: usize,
}

impl RouteLatencyStats {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(cap.min(512))),
            cap: cap.max(16),
        }
    }

    pub fn record(&self, total_ms: u64) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        if guard.len() >= self.cap {
            guard.pop_front();
        }
        guard.push_back(total_ms);
    }

    pub fn p95_ms(&self) -> u64 {
        let Ok(guard) = self.inner.lock() else {
            return 0;
        };
        if guard.is_empty() {
            return 0;
        }
        let mut vals: Vec<u64> = guard.iter().copied().collect();
        vals.sort_unstable();
        let idx = ((vals.len() as f64) * 0.95).ceil() as usize;
        vals[idx.saturating_sub(1).min(vals.len() - 1)]
    }
}

impl RouteTiming {
    pub fn log_line(&self, p95_ms: u64, phase: &str) {
        tracing::info!(
            target: "agent_brain::route",
            cache_hit = self.cache_hit,
            embed_cache_hit = self.embed_cache_hit,
            cache_warm = self.cache_warm,
            total_ms = self.total_us / 1000,
            embed_ms = self.embed_us / 1000,
            score_ms = self.score_us / 1000,
            build_ms = self.build_us / 1000,
            candidates = self.candidates,
            index_total = self.index_total,
            p95_ms = p95_ms,
            phase = phase,
            "route_task"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p95_tracks_recent_samples() {
        let stats = RouteLatencyStats::new(100);
        for ms in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            stats.record(ms);
        }
        assert!(stats.p95_ms() >= 90);
    }
}
