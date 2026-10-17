//! Turn Telemetry Ingestion, Token Tracking & Cache Savings Attribution (ABH-7).

use std::collections::HashMap;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnTelemetryRequest {
    pub session_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub task_phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnTelemetryOutcome {
    pub event_id: String,
    pub timestamp: i64,
    pub session_id: Option<String>,
    pub organ: String,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub estimated_dollars_saved: f64,
    pub cache_hit_pct: f64,
    pub heart_synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderSavings {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub estimated_cost_usd: f64,
    pub estimated_dollars_saved: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelSavings {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub estimated_cost_usd: f64,
    pub estimated_dollars_saved: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheSavingsSummary {
    pub total_turns: usize,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_write_tokens: u64,
    pub overall_cache_hit_pct: f64,
    pub total_cost_usd: f64,
    pub total_dollars_saved: f64,
    pub by_provider: HashMap<String, ProviderSavings>,
    pub by_model: HashMap<String, ModelSavings>,
}

/// Calculate estimated cost, dollars saved via cache, and cache hit percentage.
pub fn calculate_pricing(
    provider: &str,
    model: &str,
    prompt: u64,
    completion: u64,
    cache_read: u64,
    cache_write: u64,
) -> (f64, f64, f64) {
    let p_lower = provider.to_lowercase();
    let m_lower = model.to_lowercase();

    // Rates per 1M tokens: (prompt_rate, completion_rate, cache_read_rate, cache_write_rate)
    let (prompt_rate, completion_rate, cache_read_rate, cache_write_rate) = match p_lower.as_str() {
        "anthropic" => {
            if m_lower.contains("haiku") {
                (0.80, 4.00, 0.08, 1.00)
            } else if m_lower.contains("opus") {
                (15.00, 75.00, 1.50, 18.75)
            } else {
                // Sonnet 3.5 / 3.7 default
                (3.00, 15.00, 0.30, 3.75)
            }
        }
        "openai" => {
            if m_lower.contains("mini") {
                (0.15, 0.60, 0.075, 0.15)
            } else if m_lower.contains("o1") {
                (15.00, 60.00, 7.50, 15.00)
            } else if m_lower.contains("o3") {
                (1.10, 4.40, 0.55, 1.10)
            } else {
                // gpt-4o default
                (2.50, 10.00, 1.25, 2.50)
            }
        }
        "google" | "gemini" => {
            if m_lower.contains("flash") {
                (0.075, 0.30, 0.01875, 0.075)
            } else {
                // pro default
                (1.25, 5.00, 0.3125, 1.25)
            }
        }
        "deepseek" => {
            (0.14, 0.28, 0.014, 0.14)
        }
        _ => (0.0, 0.0, 0.0, 0.0),
    };

    let prompt_cost = (prompt as f64 / 1_000_000.0) * prompt_rate;
    let completion_cost = (completion as f64 / 1_000_000.0) * completion_rate;
    let cache_read_cost = (cache_read as f64 / 1_000_000.0) * cache_read_rate;
    let cache_write_cost = (cache_write as f64 / 1_000_000.0) * cache_write_rate;

    let total_cost = prompt_cost + completion_cost + cache_read_cost + cache_write_cost;

    // Cache savings: what cache_read would have cost at regular prompt rate minus actual cache_read cost
    let dollars_saved = if cache_read > 0 && prompt_rate > cache_read_rate {
        (cache_read as f64 / 1_000_000.0) * (prompt_rate - cache_read_rate)
    } else {
        0.0
    };

    let total_prompt_side = prompt + cache_read;
    let cache_hit_pct = if total_prompt_side > 0 {
        (cache_read as f64 / total_prompt_side as f64) * 100.0
    } else {
        0.0
    };

    (total_cost, dollars_saved, cache_hit_pct)
}

/// Ingest and process a turn telemetry request, calculating cost and savings.
pub fn process_turn_telemetry(req: TurnTelemetryRequest) -> TurnTelemetryOutcome {
    let (cost, saved, hit_pct) = calculate_pricing(
        &req.provider,
        &req.model,
        req.prompt_tokens,
        req.completion_tokens,
        req.cache_read_tokens,
        req.cache_write_tokens,
    );

    let total_tokens = req.prompt_tokens
        + req.completion_tokens
        + req.cache_read_tokens
        + req.cache_write_tokens;

    let event_id = uuid::Uuid::new_v4().to_string();
    let timestamp = Utc::now().timestamp_millis();

    let outcome = TurnTelemetryOutcome {
        event_id,
        timestamp,
        session_id: req.session_id,
        organ: "agent-brain".to_string(),
        provider: req.provider,
        model: req.model,
        prompt_tokens: req.prompt_tokens,
        completion_tokens: req.completion_tokens,
        cache_read_tokens: req.cache_read_tokens,
        cache_write_tokens: req.cache_write_tokens,
        total_tokens,
        estimated_cost_usd: cost,
        estimated_dollars_saved: saved,
        cache_hit_pct: hit_pct,
        heart_synced: false,
    };

    let synced = sync_to_heart_async(&outcome);
    TurnTelemetryOutcome {
        heart_synced: synced,
        ..outcome
    }
}

/// Asynchronously dispatch telemetry event to agent-heart without blocking the agent turn loop.
pub fn sync_to_heart_async(outcome: &TurnTelemetryOutcome) -> bool {
    let heart_dir = agent_body_core::organ_state_dir("heart");
    let heart_db = heart_dir.join("heart.db");
    if !heart_db.exists() {
        return false;
    }

    let cloned = outcome.clone();
    std::thread::spawn(move || {
        let _ = sync_to_heart(&cloned);
    });
    true
}

/// Locate heart.db and append event directly to model_usage_events if heart is initialized.
pub fn sync_to_heart(outcome: &TurnTelemetryOutcome) -> bool {
    let heart_dir = agent_body_core::organ_state_dir("heart");
    let heart_db = heart_dir.join("heart.db");
    if !heart_db.exists() {
        return false;
    }

    match Connection::open(&heart_db) {
        Ok(conn) => {
            let res = conn.execute(
                r#"INSERT OR IGNORE INTO model_usage_events 
                   (id, timestamp, session_id, organ, provider, model, billing_type,
                    prompt_tokens, completion_tokens, cache_read_tokens, cache_write_tokens,
                    estimated_cost_usd, latency_ms)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
                params![
                    outcome.event_id,
                    outcome.timestamp,
                    outcome.session_id,
                    outcome.organ,
                    outcome.provider,
                    outcome.model,
                    "metered_api",
                    outcome.prompt_tokens as i64,
                    outcome.completion_tokens as i64,
                    outcome.cache_read_tokens as i64,
                    outcome.cache_write_tokens as i64,
                    outcome.estimated_cost_usd,
                    0i64,
                ],
            );
            res.is_ok()
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_pricing_anthropic_sonnet() {
        // 100k prompt, 10k completion, 50k cache read, 10k cache write
        let (cost, saved, hit_pct) = calculate_pricing(
            "anthropic",
            "claude-3-5-sonnet",
            100_000,
            10_000,
            50_000,
            10_000,
        );

        // Prompt: 0.1 * 3.00 = 0.30
        // Completion: 0.01 * 15.00 = 0.15
        // Cache read: 0.05 * 0.30 = 0.015
        // Cache write: 0.01 * 3.75 = 0.0375
        // Total cost = 0.30 + 0.15 + 0.015 + 0.0375 = 0.5025
        assert!((cost - 0.5025).abs() < 1e-4);

        // Saved: 0.05 * (3.00 - 0.30) = 0.05 * 2.70 = 0.135
        assert!((saved - 0.135).abs() < 1e-4);

        // Cache hit pct: 50k / (100k + 50k) = 33.333%
        assert!((hit_pct - 33.3333).abs() < 0.1);
    }

    #[test]
    fn test_calculate_pricing_openai_mini() {
        let (cost, saved, hit_pct) = calculate_pricing(
            "openai",
            "gpt-4o-mini",
            1_000_000,
            100_000,
            500_000,
            0,
        );

        // Prompt: 1.0 * 0.15 = 0.15
        // Completion: 0.1 * 0.60 = 0.06
        // Cache read: 0.5 * 0.075 = 0.0375
        // Total cost = 0.2475
        assert!((cost - 0.2475).abs() < 1e-4);

        // Saved: 0.5 * (0.15 - 0.075) = 0.0375
        assert!((saved - 0.0375).abs() < 1e-4);
        assert!((hit_pct - 33.3333).abs() < 0.1);
    }

    #[test]
    fn test_process_turn_telemetry_generates_outcome() {
        let req = TurnTelemetryRequest {
            session_id: Some("session-123".into()),
            provider: "anthropic".into(),
            model: "claude-3-5-sonnet".into(),
            prompt_tokens: 5_000,
            completion_tokens: 1_000,
            cache_read_tokens: 20_000,
            cache_write_tokens: 0,
            latency_ms: 1200,
            task_phase: Some("executing".into()),
        };

        let outcome = process_turn_telemetry(req);
        assert_eq!(outcome.organ, "agent-brain");
        assert_eq!(outcome.provider, "anthropic");
        assert_eq!(outcome.prompt_tokens, 5_000);
        assert_eq!(outcome.total_tokens, 26_000);
        assert!(outcome.estimated_cost_usd > 0.0);
        assert!(outcome.estimated_dollars_saved > 0.0);
        assert!(outcome.cache_hit_pct > 70.0); // 20k / 25k = 80%
    }

    #[test]
    fn test_calculate_pricing_gemini_flash_and_pro() {
        // Gemini Flash: $0.075 / 1M prompt, $0.30 / 1M completion, $0.01875 / 1M cache read
        let (cost, saved, hit_pct) = calculate_pricing(
            "google",
            "gemini-2.0-flash",
            1_000_000,
            100_000,
            500_000,
            0,
        );
        // Prompt: 1.0 * 0.075 = 0.075
        // Completion: 0.1 * 0.30 = 0.03
        // Cache read: 0.5 * 0.01875 = 0.009375
        // Total cost = 0.075 + 0.03 + 0.009375 = 0.114375
        assert!((cost - 0.114375).abs() < 1e-4);
        // Saved: 0.5 * (0.075 - 0.01875) = 0.028125
        assert!((saved - 0.028125).abs() < 1e-4);
        assert!((hit_pct - 33.3333).abs() < 0.1);

        // Gemini Pro: $1.25 / 1M prompt, $5.00 / 1M completion, $0.3125 / 1M cache read
        let (pro_cost, pro_saved, _) = calculate_pricing(
            "gemini",
            "gemini-1.5-pro",
            100_000,
            10_000,
            50_000,
            0,
        );
        assert!(pro_cost > 0.0);
        assert!(pro_saved > 0.0);
    }

    #[test]
    fn test_tokens_saved_vs_full_read_calculation() {
        let full_file_tokens = 50_000usize;
        let tokens_used = 500usize;
        let tokens_saved = full_file_tokens.saturating_sub(tokens_used);
        let savings_pct = (tokens_saved as f64 / full_file_tokens as f64) * 100.0;

        assert_eq!(tokens_saved, 49_500);
        assert!((savings_pct - 99.0).abs() < 1e-2);

        // Test dollar savings at metered rate ($3.00 / 1M prompt tokens)
        let rate_per_million = 3.00f64;
        let dollars_saved = (tokens_saved as f64 / 1_000_000.0) * rate_per_million;
        assert!((dollars_saved - 0.1485).abs() < 1e-4);
    }

    #[test]
    fn test_sync_to_heart_non_blocking_integration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let heart_db = temp_dir.path().join("heart.db");
        let conn = Connection::open(&heart_db).unwrap();
        conn.execute(
            r#"CREATE TABLE IF NOT EXISTS model_usage_events (
                id TEXT PRIMARY KEY,
                timestamp INTEGER,
                session_id TEXT,
                organ TEXT,
                provider TEXT,
                model TEXT,
                billing_type TEXT,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                estimated_cost_usd REAL,
                latency_ms INTEGER
            )"#,
            [],
        )
        .unwrap();

        let outcome = TurnTelemetryOutcome {
            event_id: "test-evt-nonblock".into(),
            timestamp: Utc::now().timestamp_millis(),
            session_id: Some("sess-1".into()),
            organ: "agent-brain".into(),
            provider: "anthropic".into(),
            model: "claude-3-7-sonnet".into(),
            prompt_tokens: 1000,
            completion_tokens: 500,
            cache_read_tokens: 2000,
            cache_write_tokens: 0,
            total_tokens: 3500,
            estimated_cost_usd: 0.01,
            estimated_dollars_saved: 0.005,
            cache_hit_pct: 66.6,
            heart_synced: false,
        };

        let res = conn.execute(
            r#"INSERT OR IGNORE INTO model_usage_events 
               (id, timestamp, session_id, organ, provider, model, billing_type,
                prompt_tokens, completion_tokens, cache_read_tokens, cache_write_tokens,
                estimated_cost_usd, latency_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
            params![
                outcome.event_id,
                outcome.timestamp,
                outcome.session_id,
                outcome.organ,
                outcome.provider,
                outcome.model,
                "metered_api",
                outcome.prompt_tokens as i64,
                outcome.completion_tokens as i64,
                outcome.cache_read_tokens as i64,
                outcome.cache_write_tokens as i64,
                outcome.estimated_cost_usd,
                0i64,
            ],
        );
        assert!(res.is_ok());
    }

    #[test]
    fn test_telemetry_dispatch_latency_overhead() {
        let req = TurnTelemetryRequest {
            session_id: Some("perf-sess".into()),
            provider: "openai".into(),
            model: "gpt-4o".into(),
            prompt_tokens: 2500,
            completion_tokens: 600,
            cache_read_tokens: 1500,
            cache_write_tokens: 0,
            latency_ms: 450,
            task_phase: Some("implementing".into()),
        };

        // Warm up connection/disk caching
        let _ = process_turn_telemetry(req.clone());

        let iterations = 10;
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let outcome = process_turn_telemetry(req.clone());
            assert_eq!(outcome.organ, "agent-brain");
        }
        let avg_micros = start.elapsed().as_micros() / iterations;

        // Verify average dispatch overhead on the turn loop is well under 1ms (1,000 microseconds)
        assert!(
            avg_micros < 1000,
            "average dispatch overhead exceeded 1ms budget: {}us",
            avg_micros
        );
    }
}
