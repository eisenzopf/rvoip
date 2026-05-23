//! Connection envelope payloads per CONVERSATION_PROTOCOL.md §7.4.

use serde::{Deserialize, Serialize};

/// `connection.offer` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionOffer {
    pub by_participant: String,
    pub substrate: String,
    pub capabilities: serde_json::Value,
    pub streams_offered: Vec<StreamOffer>,
    #[serde(default)]
    pub substrate_setup: serde_json::Value,
}

/// `connection.answer` (bidi) payload. Mirrors `connection.offer`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionAnswer {
    pub by_participant: String,
    pub substrate: String,
    pub capabilities: serde_json::Value,
    pub streams_answered: Vec<StreamAnswer>,
    #[serde(default)]
    pub substrate_setup: serde_json::Value,
}

/// `connection.update` (bidi) payload — hold, resume, mute, codec-renegotiate, etc.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionUpdate {
    pub action: String,
    #[serde(default)]
    pub streams: Vec<String>,
    #[serde(default)]
    pub codec_preferences: Vec<String>,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// `connection.end` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionEnd {
    pub reason_code: u16,
    pub reason: String,
}

/// `connection.quality` (bidi) payload — per-Stream quality snapshot.
///
/// CONVERSATION_PROTOCOL.md §10.3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionQuality {
    pub interval_ms: u32,
    pub streams: Vec<StreamQuality>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamOffer {
    pub id: String,
    pub kind: String,
    pub direction: String,
    pub codec_preferences: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamAnswer {
    pub id: String,
    pub kind: String,
    pub direction: String,
    pub codec: serde_json::Value,
}

/// `substrate_setup` payload when `substrate = "websocket+webrtc"`.
///
/// Per CONVERSATION_PROTOCOL.md §10.2.1. The full SDP carries ICE
/// candidates + DTLS fingerprint inline (no trickle ICE in v0).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebRtcSubstrateSetup {
    /// Always `"websocket+webrtc"`.
    pub kind: String,
    /// Complete SDP offer (in `connection.offer`) or answer
    /// (in `connection.answer`).
    pub sdp: String,
}

impl WebRtcSubstrateSetup {
    /// Convenience constructor that sets `kind` correctly.
    pub fn new(sdp: impl Into<String>) -> Self {
        Self {
            kind: "websocket+webrtc".into(),
            sdp: sdp.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamQuality {
    pub strm_id: String,
    pub loss_pct: f32,
    pub jitter_ms: u32,
    pub rtt_ms: u32,
    pub mos: f32,
    pub bitrate_bps: u32,
    pub packets_sent: u64,
    pub packets_received: u64,
}
