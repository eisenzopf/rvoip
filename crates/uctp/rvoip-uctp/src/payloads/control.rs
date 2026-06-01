//! Control / DTMF / identity / error / ack payloads.
//!
//! Per CONVERSATION_PROTOCOL.md §7.5 (DTMF), §5.6/§5.8 (identity), §11
//! (error/ack).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// --- DTMF (§7.5) ---

/// `dtmf.send` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DtmfSend {
    pub digits: String,
    pub duration_ms: u32,
    /// `"rfc4733"` or `"info"` — gateway translates to RFC 2833 / SIP INFO.
    pub method: String,
}

/// `dtmf.received` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DtmfReceived {
    pub digits: String,
    pub duration_ms: u32,
    pub received_at: DateTime<Utc>,
    pub source: String,
}

// --- Identity (§5.6, §5.8) ---

/// `identity.assurance-changed` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityAssuranceChanged {
    pub previous: String,
    pub current: String,
    pub reason: String,
}

/// `identity.step-up-request` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityStepUpRequest {
    pub required: String,
    #[serde(default)]
    pub allowed_methods: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `identity.step-up-response` (C→S) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityStepUpResponse {
    pub method: String,
    pub credential: String,
}

// --- Errors / control (§11) ---

/// `error` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Error {
    pub code: u16,
    /// `protocol` | `auth` | `media` | `policy` | `transient`.
    pub category: String,
    pub reason: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// `ack` (bidi) payload — advisory generic acknowledgment.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ack {
    #[serde(default)]
    pub details: serde_json::Value,
}

// --- vCon (§7.6) ---

/// `recording.vcon-ready` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingVconReady {
    pub vcon_handle: serde_json::Value,
    pub encrypted: bool,
    pub signed_by: Vec<String>,
}

/// `recording.vcon-fetch` (C→S) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingVconFetch {
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

/// `recording.vcon-fetched` (S→C, response) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
