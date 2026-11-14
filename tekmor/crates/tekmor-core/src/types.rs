use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedRecipe {
    pub cmd: String,
    pub exit_code: i32,
    pub source: String,
    pub recorded_at: i64,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationStatus {
    pub cmd: String,
    pub exit_code: i32,
    pub recorded_at: i64,
    pub passing: bool,
}

