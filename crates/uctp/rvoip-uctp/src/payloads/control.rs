//! Control / DTMF / identity / error / ack payloads.
//!
//! Per CONVERSATION_PROTOCOL.md §7.5 (DTMF), §5.6/§5.8 (identity), §11
//! (error/ack).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

// --- DTMF (§7.5) ---

/// `dtmf.send` (bidi) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct DtmfSend {
    pub digits: String,
    pub duration_ms: u32,
    /// `"rfc4733"` or `"info"` — gateway translates to RFC 2833 / SIP INFO.
    pub method: String,
}

impl fmt::Debug for DtmfSend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DtmfSend")
            .field("digit_count", &self.digits.chars().count())
            .field("duration_ms", &self.duration_ms)
            .field("method_bytes", &self.method.len())
            .finish()
    }
}

/// `dtmf.received` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct DtmfReceived {
    pub digits: String,
    pub duration_ms: u32,
    pub received_at: DateTime<Utc>,
    pub source: String,
}

impl fmt::Debug for DtmfReceived {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DtmfReceived")
            .field("digit_count", &self.digits.chars().count())
            .field("duration_ms", &self.duration_ms)
            .field("received_at_present", &true)
            .field("source_bytes", &self.source.len())
            .finish()
    }
}

// --- Identity (§5.6, §5.8) ---

/// `identity.assurance-changed` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct IdentityAssuranceChanged {
    pub previous: String,
    pub current: String,
    pub reason: String,
}

impl fmt::Debug for IdentityAssuranceChanged {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityAssuranceChanged")
            .field("previous_bytes", &self.previous.len())
            .field("current_bytes", &self.current.len())
            .field("reason_bytes", &self.reason.len())
            .finish()
    }
}

/// `identity.step-up-request` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct IdentityStepUpRequest {
    pub required: String,
    #[serde(default)]
    pub allowed_methods: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl fmt::Debug for IdentityStepUpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityStepUpRequest")
            .field("required_bytes", &self.required.len())
            .field("allowed_method_count", &self.allowed_methods.len())
            .field("reason_present", &self.reason.is_some())
            .field("reason_bytes", &self.reason.as_ref().map_or(0, String::len))
            .finish()
    }
}

/// `identity.step-up-response` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct IdentityStepUpResponse {
    pub method: String,
    pub credential: String,
}

impl fmt::Debug for IdentityStepUpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityStepUpResponse")
            .field("method_present", &!self.method.is_empty())
            .field("method_bytes", &self.method.len())
            .field("credential_present", &!self.credential.is_empty())
            .field("credential_bytes", &self.credential.len())
            .finish()
    }
}

// --- Errors / control (§11) ---

/// `error` (bidi) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct Error {
    pub code: u16,
    /// `protocol` | `auth` | `media` | `policy` | `transient`.
    pub category: String,
    pub reason: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("code", &self.code)
            .field("category_bytes", &self.category.len())
            .field("reason_present", &!self.reason.is_empty())
            .field("reason_bytes", &self.reason.len())
            .field("detail_kind", &json_kind(&self.details))
            .finish()
    }
}

/// `ack` (bidi) payload — advisory generic acknowledgment.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Ack {
    #[serde(default)]
    pub details: serde_json::Value,
}

impl fmt::Debug for Ack {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Ack")
            .field("detail_kind", &json_kind(&self.details))
            .finish()
    }
}

// --- vCon (§7.6) ---

/// `recording.vcon-ready` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingVconReady {
    pub vcon_handle: serde_json::Value,
    pub encrypted: bool,
    pub signed_by: Vec<String>,
}

impl fmt::Debug for RecordingVconReady {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingVconReady")
            .field("handle_kind", &json_kind(&self.vcon_handle))
            .field("encrypted", &self.encrypted)
            .field("signer_count", &self.signed_by.len())
            .finish()
    }
}

/// `recording.vcon-fetch` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingVconFetch {
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

impl fmt::Debug for RecordingVconFetch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingVconFetch")
            .field("uuid_present", &self.uuid.is_some())
            .field("url_present", &self.url.is_some())
            .field("content_hash_present", &self.content_hash.is_some())
            .finish()
    }
}

/// `recording.vcon-fetched` (S→C, response) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct RecordingVconFetched {
    /// `"inline"` or `"url"`.
    pub delivery: String,
    #[serde(default)]
    pub vcon: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for RecordingVconFetched {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingVconFetched")
            .field("delivery_bytes", &self.delivery.len())
            .field("inline_vcon_present", &self.vcon.is_some())
            .field(
                "inline_vcon_bytes",
                &self.vcon.as_ref().map_or(0, String::len),
            )
            .field("url_present", &self.url.is_some())
            .field("expires_at_present", &self.expires_at.is_some())
            .finish()
    }
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
