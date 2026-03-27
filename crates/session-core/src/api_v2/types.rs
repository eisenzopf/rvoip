//! Core API Types (v2)

use std::time::Instant;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::errors_v2::Result;
use std::fmt;
use crate::state_table::CallState;

pub use rvoip_sip_core::StatusCode;
pub use crate::errors_v2::SessionError as Error;

/// Unique identifier for a session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        Self(format!("sess_{}", Uuid::new_v4()))
    }

    pub fn from_string(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Role of a session in a SIP dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionRole {
    UAC,
    UAS,
}

pub type Session = CallSession;

/// Represents a prepared outgoing call
#[derive(Debug, Clone)]
pub struct PreparedCall {
    pub session_id: SessionId,
    pub from: String,
    pub to: String,
    pub sdp_offer: String,
    pub local_rtp_port: u16,
}

/// Represents an active call session
#[derive(Debug, Clone)]
pub struct CallSession {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub started_at: Option<Instant>,
    pub sip_call_id: Option<String>,
}

impl CallSession {
    pub fn id(&self) -> &SessionId { &self.id }
    pub fn state(&self) -> &CallState { &self.state }
    pub fn is_active(&self) -> bool { matches!(self.state, CallState::Active) }
}

/// Represents an incoming call that needs to be handled
#[derive(Debug, Clone)]
pub struct IncomingCall {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub sdp: Option<String>,
    pub headers: std::collections::HashMap<String, String>,
    pub received_at: Instant,
    pub sip_call_id: Option<String>,
}

impl IncomingCall {
    pub fn caller(&self) -> &str { &self.from }
    pub fn called(&self) -> &str { &self.to }
}

/// Decision on how to handle an incoming call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallDecision {
    Accept(Option<String>),
    Reject(String),
    Defer,
    Forward(String),
}

/// Statistics about active sessions
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub failed_sessions: usize,
    pub average_duration: Option<std::time::Duration>,
}

/// Media information for a session
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub codec: Option<String>,
}

/// Call direction
#[derive(Debug, Clone, PartialEq)]
pub enum CallDirection {
    Outgoing,
    Incoming,
}

/// Call termination reason
#[derive(Debug, Clone)]
pub enum TerminationReason {
    LocalHangup,
    RemoteHangup,
    Rejected(String),
    Error(String),
    Timeout,
}

/// Parsed SDP information
#[derive(Debug, Clone)]
pub struct SdpInfo {
    pub ip: String,
    pub port: u16,
    pub codecs: Vec<String>,
}

/// Parse SDP connection information
pub fn parse_sdp_connection(sdp: &str) -> Result<SdpInfo> {
    let mut ip = None;
    let mut port = None;
    let mut codecs = Vec::new();

    for line in sdp.lines() {
        if line.starts_with("c=IN IP4 ") {
            ip = line.strip_prefix("c=IN IP4 ").map(|s| s.to_string());
        } else if line.starts_with("m=audio ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 1 {
                port = parts[1].parse().ok();
            }
            if parts.len() > 3 {
                for codec in &parts[3..] {
                    codecs.push(codec.to_string());
                }
            }
        } else if line.starts_with("a=rtpmap:") {
            if let Some(codec_info) = line.strip_prefix("a=rtpmap:") {
                let parts: Vec<&str> = codec_info.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Some(codec_name) = parts[1].split('/').next() {
                        codecs.push(codec_name.to_string());
                    }
                }
            }
        }
    }

    match (ip, port) {
        (Some(ip), Some(port)) => Ok(SdpInfo { ip, port, codecs }),
        _ => Err(crate::errors_v2::SessionError::MediaIntegration {
            reason: "Failed to parse SDP connection information".to_string()
        }),
    }
}

pub use rvoip_media_core::types::AudioFrame;

/// Configuration for audio streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamConfig {
    pub sample_rate: u32,
    pub channels: u8,
    pub codec: String,
    pub frame_size_ms: u32,
    pub enable_aec: bool,
    pub enable_agc: bool,
    pub enable_vad: bool,
}

impl Default for AudioStreamConfig {
    fn default() -> Self {
        Self {
            sample_rate: 8000,
            channels: 1,
            codec: "PCMU".to_string(),
            frame_size_ms: 20,
            enable_aec: true,
            enable_agc: true,
            enable_vad: true,
        }
    }
}

impl AudioStreamConfig {
    pub fn new(sample_rate: u32, channels: u8, codec: impl Into<String>) -> Self {
        Self { sample_rate, channels, codec: codec.into(), ..Default::default() }
    }

    pub fn frame_size_samples(&self) -> usize {
        (self.sample_rate as usize * self.frame_size_ms as usize) / 1000
    }

    pub fn frame_size_bytes(&self) -> usize {
        self.frame_size_samples() * self.channels as usize * 2
    }

    pub fn telephony() -> Self { Self::default() }

    pub fn wideband() -> Self {
        Self { sample_rate: 16000, codec: "Opus".to_string(), ..Default::default() }
    }

    pub fn high_quality() -> Self {
        Self { sample_rate: 48000, channels: 2, codec: "Opus".to_string(), ..Default::default() }
    }
}
