use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use crate::keep_alive::KeepAlive;

/// Error returned when a caller supplies an invalid decision or generation request shape or size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidRequest(pub String);

impl fmt::Display for InvalidRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub state: serde_json::Value,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub questions: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// How long the model should stay loaded after this request (Ollama semantics:
    /// the most recent request wins; omitted means the server default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<KeepAlive>,
}

fn default_temperature() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionSpec {
    #[serde(rename = "type", default = "default_q_type")]
    pub q_type: String,
    pub instructions: Option<serde_json::Value>,
    pub criteria: Option<serde_json::Value>,
    pub options: Option<serde_json::Value>,
    pub choices: Option<serde_json::Value>,
    pub levels: Option<serde_json::Value>,
}

fn default_q_type() -> String {
    "choice".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UnifiedResponse {
    Jev(JevResponse),
    Flat(DecisionResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDecisionRequest {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub questions: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub items: Vec<BatchItem>,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// How long the model should stay loaded after this request (Ollama semantics:
    /// the most recent request wins; omitted means the server default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<KeepAlive>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItem {
    pub id: String,
    #[serde(default)]
    pub state: serde_json::Value,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub questions: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItemResult {
    pub id: String,
    #[serde(flatten)]
    pub response: UnifiedResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDecisionResponse {
    pub model: String,
    pub results: Vec<BatchItemResult>,
    pub total_tokens: usize,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevResponse {
    pub model: String,
    pub answers: serde_json::Map<String, serde_json::Value>,
    pub usage: UsageInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnswerOutput {
    #[serde(rename = "type")]
    pub q_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noul: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    pub probabilities: HashMap<String, f32>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: usize,
    pub output_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub decision: String,
    pub confidence: f32,
    pub probabilities: HashMap<String, f32>,
    pub model: String,
    pub latency_ms: f64,
    pub tokens_evaluated: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceItem {
    pub key: String,
    pub display: String,
}
