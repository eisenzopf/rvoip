//! Type definitions for the client-core library
//! 
//! This module contains all data structures and types used throughout
//! the client-core library, extracted from manager.rs for better organization.

use chrono::{DateTime, Utc};
use crate::call::CallId;

// ===== CORE CLIENT TYPES =====

/// Statistics about the client
#[derive(Debug, Clone)]
pub struct ClientStats {
    pub is_running: bool,
    pub local_sip_addr: std::net::SocketAddr,
    pub local_media_addr: std::net::SocketAddr,
    pub total_calls: usize,
    pub connected_calls: usize,
    pub total_registrations: usize,
    pub active_registrations: usize,
}

// ===== MEDIA-RELATED TYPES =====

/// Media information for a specific call
#[derive(Debug, Clone)]
pub struct CallMediaInfo {
    pub call_id: CallId,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub codec: Option<String>,
    pub is_muted: bool,
    pub is_on_hold: bool,
    pub audio_direction: AudioDirection,
    pub quality_metrics: Option<AudioQualityMetrics>,
}

/// Audio codec information
#[derive(Debug, Clone)]
pub struct AudioCodecInfo {
    pub name: String,
    pub payload_type: u8,
    pub clock_rate: u32,
    pub channels: u8,
    pub description: String,
    pub quality_rating: u8, // 1-5 scale
}

/// Audio direction for a call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioDirection {
    SendReceive,
    SendOnly,
    ReceiveOnly,
    Inactive,
}

/// Audio quality metrics
#[derive(Debug, Clone)]
pub struct AudioQualityMetrics {
    pub mos_score: Option<f32>,      // Mean Opinion Score (1.0-5.0)
    pub jitter_ms: Option<u32>,      // Jitter in milliseconds
    pub packet_loss_percent: Option<f32>, // Packet loss percentage
    pub round_trip_time_ms: Option<u32>,  // RTT in milliseconds
    pub bitrate_kbps: Option<u32>,   // Current bitrate in kbps
}

/// Comprehensive media capabilities
#[derive(Debug, Clone)]
pub struct MediaCapabilities {
    pub supported_codecs: Vec<AudioCodecInfo>,
    pub can_hold: bool,
    pub can_mute_microphone: bool,
    pub can_mute_speaker: bool,
    pub can_send_dtmf: bool,
    pub can_transfer: bool,
    pub supports_sdp_offer_answer: bool,
    pub supports_rtp: bool,
    pub supports_rtcp: bool,
    pub max_concurrent_calls: usize,
    pub supported_media_types: Vec<String>, // "audio", "video"
}

/// Represents the capabilities available for a call in its current state
#[derive(Debug, Clone)]
pub struct CallCapabilities {
    pub can_hold: bool,
    pub can_resume: bool,
    pub can_transfer: bool,
    pub can_send_dtmf: bool,
    pub can_mute: bool,
    pub can_hangup: bool,
}

impl Default for CallCapabilities {
    fn default() -> Self {
        Self {
            can_hold: false,
            can_resume: false,
            can_transfer: false,
            can_send_dtmf: false,
            can_mute: false,
            can_hangup: false,
        }
    }
}

/// Information about an active media session
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    pub call_id: CallId,
    pub session_id: rvoip_session_core::api::types::SessionId,
    pub media_session_id: String,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub codec: Option<String>,
    pub media_direction: AudioDirection,
    pub quality_metrics: Option<AudioQualityMetrics>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Negotiated media parameters between endpoints
#[derive(Debug, Clone)]
pub struct NegotiatedMediaParams {
    pub call_id: CallId,
    pub negotiated_codec: Option<String>,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub audio_direction: AudioDirection,
    pub local_sdp: String,
    pub remote_sdp: String,
    pub negotiated_at: DateTime<Utc>,
    pub supports_dtmf: bool,
    pub supports_hold: bool,
    pub bandwidth_kbps: Option<u32>,
    pub encryption_enabled: bool,
}

/// Enhanced media capabilities including advanced features
#[derive(Debug, Clone)]
pub struct EnhancedMediaCapabilities {
    pub basic_capabilities: MediaCapabilities,
    pub supports_sdp_offer_answer: bool,
    pub supports_media_session_lifecycle: bool,
    pub supports_sdp_renegotiation: bool,
    pub supports_early_media: bool,
    pub supports_media_session_updates: bool,
    pub supports_codec_negotiation: bool,
    pub supports_bandwidth_management: bool,
    pub supports_encryption: bool,
    pub supported_sdp_version: String,
    pub max_media_sessions: usize,
    pub preferred_rtp_port_range: (u16, u16),
    pub supported_transport_protocols: Vec<String>,
}
