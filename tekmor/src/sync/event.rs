//! Bundle v2 sync event envelope and payloads.

use serde::{Deserialize, Serialize};

use super::hlc::Hlc;

pub const SCHEMA_VERSION_V2: u32 = 2;
pub const MIN_READER_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub hlc: Hlc,
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    #[serde(default = "default_domain")]
    pub domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<serde_json::Value>,
    pub payload: serde_json::Value,
}

fn default_domain() -> String {
    "unclassified".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactPutPayload {
    pub topic: String,
    pub fact: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
    pub confidence: f64,
    #[serde(default = "positive")]
    pub polarity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_when: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<i64>,
    pub content_hash: String,
    #[serde(default)]
    pub source: String,
}

fn positive() -> String {
    "positive".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactTombstonePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootManifestV2 {
    pub schema_version: u32,
    #[serde(default = "min_reader")]
    pub min_reader_version: u32,
    pub devices: Vec<String>,
    #[serde(default)]
    pub migrated_at: Option<i64>,
}

fn min_reader() -> u32 {
    MIN_READER_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceManifestV2 {
    pub device_id: String,
    pub hlc: Hlc,
    pub event_count: usize,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}
