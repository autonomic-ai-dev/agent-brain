//! Ebbinghaus forgetting-curve weight decay for memory retrieval.

/// Retention multiplier in `(0, 1]` — higher when recently reinforced.
///
/// `stability_days` grows with confidence and useful retrieval feedback.
#[must_use]
pub fn ebbinghaus_multiplier(
    age_ms: i64,
    confidence: f64,
    useful_count: u32,
    now_ms: i64,
) -> f64 {
    if age_ms <= 0 {
        return 1.0;
    }
    let elapsed_days = ((now_ms - age_ms).max(0) as f64) / (24.0 * 3600.0 * 1000.0);
    let stability = 30.0 + confidence.clamp(0.0, 1.0) * 60.0 + f64::from(useful_count) * 15.0;
    (-elapsed_days / stability.max(1.0)).exp()
}

/// Apply temporal decay to a stored fact confidence (Phase 6.1).
#[must_use]
pub fn decay_stored_confidence(confidence: f64, updated_at_ms: i64, now_ms: i64) -> f64 {
    (confidence * ebbinghaus_multiplier(updated_at_ms, confidence, 0, now_ms)).clamp(0.05, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_memory_retains_full_weight() {
        let now = 1_700_000_000_000_i64;
        assert!((ebbinghaus_multiplier(now, 0.9, 0, now) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn old_memory_decays_without_reinforcement() {
        let now = 1_700_000_000_000_i64;
        let ninety_days_ms = 90_i64 * 24 * 3600 * 1000;
        let m = ebbinghaus_multiplier(now - ninety_days_ms, 0.5, 0, now);
        assert!(m < 0.5, "expected decay, got {m}");
    }

    #[test]
    fn useful_count_slows_decay() {
        let now = 1_700_000_000_000_i64;
        let thirty_days_ms = 30_i64 * 24 * 3600 * 1000;
        let weak = ebbinghaus_multiplier(now - thirty_days_ms, 0.5, 0, now);
        let strong = ebbinghaus_multiplier(now - thirty_days_ms, 0.5, 5, now);
        assert!(strong > weak);
    }
}
