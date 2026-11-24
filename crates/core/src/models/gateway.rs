use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An API-key account for an Anthropic- or OpenAI-compatible inference
/// service. Unlike a subscription it has no sign-in or quota; the agent
/// (Claude Code or Codex) runs against the service with the stored key.
/// The key lives in the macOS Keychain, never in this record.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayRecord {
    pub id: Uuid,
    /// Service name, such as `melious`. It must not be a provider name.
    pub service: String,
    pub slug: String,
    /// Root of the Anthropic Messages API, as Claude Code expects it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_base_url: Option<String>,
    /// Base of the OpenAI API including `/v1`, as Codex expects it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,
    /// Model for Claude Code's Opus/Sonnet aliases and Codex.
    pub big_model: String,
    /// Model for Claude Code's Haiku alias and background tasks.
    pub small_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<GatewayHealth>,
}

impl GatewayRecord {
    pub fn reference(&self) -> String {
        format!("{}/{}", self.service, self.slug)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GatewayStatus {
    /// The key is accepted and both models answered.
    Ok,
    /// The key works but a model is missing, busy or failing.
    Degraded,
    /// The service is unreachable, rejects the key, or no model answers.
    Down,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHealth {
    pub status: GatewayStatus,
    pub checked_at: DateTime<Utc>,
    pub latency_ms: u64,
    /// Why the status is not `ok`; absent when everything answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Defaults for a known service so `api add melious` needs only a key.
pub struct GatewayPreset {
    pub anthropic_base_url: &'static str,
    pub openai_base_url: &'static str,
    pub big_model: &'static str,
    pub small_model: &'static str,
}

pub fn gateway_preset(service: &str) -> Option<GatewayPreset> {
    match service {
        "melious" => Some(GatewayPreset {
            anthropic_base_url: "https://api.melious.ai",
            openai_base_url: "https://api.melious.ai/v1",
            big_model: "glm-5.3",
            small_model: "glm-5.3-flash",
        }),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct AddGatewayOptions {
    pub slug: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub openai_base_url: Option<String>,
    pub big_model: Option<String>,
    pub small_model: Option<String>,
    /// Save even when the health check fails, for example while offline.
    pub skip_check: bool,
}

#[derive(Clone, Debug)]
pub struct GatewayExecution {
    pub gateway: GatewayRecord,
    pub home: std::path::PathBuf,
    pub set: std::collections::BTreeMap<String, String>,
    pub unset: Vec<String>,
}
