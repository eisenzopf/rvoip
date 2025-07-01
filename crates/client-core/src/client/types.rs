//! Type definitions for the client-core library
//! 
//! This module contains all data structures and types used throughout
//! the client-core library, providing a comprehensive set of types for
//! VoIP client operations including calls, media, capabilities, and statistics.
//! 
//! # Type Categories
//! 
//! - **Core Client Types** - Basic client information and statistics
//! - **Media Types** - Audio, codecs, quality metrics, and capabilities
//! - **Call Types** - Call-specific information and capabilities
//! - **Session Types** - Media session and negotiation parameters
//! 
//! # Usage Examples
//! 
//! ## Getting Client Statistics
//! 
//! ```rust
//! # use rvoip_client_core::{Client, ClientStats};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! let stats: ClientStats = client.get_client_stats().await;
//! println!("Running: {}, Active calls: {}", stats.is_running, stats.connected_calls);
//! # Ok(())
//! # }
//! ```
//! 
//! ## Working with Media Information
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, CallId, CallMediaInfo};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
//! let media_info: CallMediaInfo = client.get_call_media_info(&call_id).await?;
//! if let Some(codec) = &media_info.codec {
//!     println!("Using codec: {}", codec);
//! }
//! # Ok(())
//! # }
//! ```
//! 
//! ## Checking Media Capabilities
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, MediaCapabilities};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! let capabilities: MediaCapabilities = client.get_media_capabilities().await;
//! if capabilities.can_hold {
//!     println!("Hold feature is supported");
//! }
//! println!("Supported codecs: {}", capabilities.supported_codecs.len());
//! # Ok(())
//! # }
//! ```

use chrono::{DateTime, Utc};
use crate::call::CallId;

// ===== CORE CLIENT TYPES =====

/// Statistics about the client's current state and activity
/// 
/// Provides a comprehensive view of the client's operational status,
/// including network configuration and active session counts.
#[derive(Debug, Clone)]
pub struct ClientStats {
    /// Whether the client is currently running and processing events
    pub is_running: bool,
    /// Local socket address for SIP signaling
    pub local_sip_addr: std::net::SocketAddr,
    /// Local socket address for media (RTP) traffic
    pub local_media_addr: std::net::SocketAddr,
    /// Total number of calls handled since client started
    pub total_calls: usize,
    /// Number of currently connected calls
    pub connected_calls: usize,
    /// Total number of registrations attempted since client started
    pub total_registrations: usize,
    /// Number of currently active registrations
    pub active_registrations: usize,
}

// ===== MEDIA-RELATED TYPES =====

/// Media information for a specific call
/// 
/// Contains all media-related details for an active call including
/// SDP information, RTP ports, codec selection, and quality metrics.
#[derive(Debug, Clone)]
pub struct CallMediaInfo {
    /// Unique identifier for the call this media info belongs to
    pub call_id: CallId,
    /// Local Session Description Protocol (SDP) offer/answer
    pub local_sdp: Option<String>,
    /// Remote Session Description Protocol (SDP) offer/answer
    pub remote_sdp: Option<String>,
    /// Local RTP port for media transmission
    pub local_rtp_port: Option<u16>,
    /// Remote RTP port for media reception
    pub remote_rtp_port: Option<u16>,
    /// Currently negotiated audio codec (e.g., "PCMU", "OPUS")
    pub codec: Option<String>,
    /// Whether the microphone is currently muted
    pub is_muted: bool,
    /// Whether the call is currently on hold
    pub is_on_hold: bool,
    /// Current audio direction for this call
    pub audio_direction: AudioDirection,
    /// Real-time quality metrics for the call (if available)
    pub quality_metrics: Option<AudioQualityMetrics>,
}

/// Information about an audio codec
/// 
/// Describes a specific audio codec including its technical parameters
/// and subjective quality rating for user selection.
#[derive(Debug, Clone)]
pub struct AudioCodecInfo {
    /// Codec name (e.g., "PCMU", "PCMA", "OPUS", "G722")
    pub name: String,
    /// RTP payload type identifier (0-127)
    pub payload_type: u8,
    /// Sampling rate in Hz (e.g., 8000, 16000, 48000)
    pub clock_rate: u32,
    /// Number of audio channels (1 for mono, 2 for stereo)
    pub channels: u8,
    /// Human-readable description of the codec
    pub description: String,
    /// Subjective quality rating on a 1-5 scale (5 being best)
    pub quality_rating: u8, // 1-5 scale
}

/// Audio direction for a call
/// 
/// Defines the permitted audio flow directions between endpoints
/// according to SDP specifications (RFC 4566).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioDirection {
    /// Audio can be sent and received (bidirectional)
    SendReceive,
    /// Audio can only be sent (transmit only)
    SendOnly,
    /// Audio can only be received (receive only)
    ReceiveOnly,
    /// No audio transmission in either direction
    Inactive,
}

/// Real-time audio quality metrics
/// 
/// Provides quantitative measurements of call quality that can be used
/// for monitoring, debugging, and user experience optimization.
#[derive(Debug, Clone)]
pub struct AudioQualityMetrics {
    /// Mean Opinion Score (1.0-5.0), higher is better quality
    pub mos_score: Option<f32>,      // Mean Opinion Score (1.0-5.0)
    /// Packet jitter in milliseconds (lower is better)
    pub jitter_ms: Option<u32>,      // Jitter in milliseconds
    /// Packet loss percentage (0.0-100.0, lower is better)
    pub packet_loss_percent: Option<f32>, // Packet loss percentage
    /// Round-trip time in milliseconds (lower is better)
    pub round_trip_time_ms: Option<u32>,  // RTT in milliseconds
    /// Current bitrate in kilobits per second
    pub bitrate_kbps: Option<u32>,   // Current bitrate in kbps
}

/// Comprehensive media capabilities of the client
/// 
/// Describes what media operations and features are supported
/// by the current client configuration and underlying system.
#[derive(Debug, Clone)]
pub struct MediaCapabilities {
    /// List of audio codecs supported by this client
    pub supported_codecs: Vec<AudioCodecInfo>,
    /// Whether calls can be placed on hold
    pub can_hold: bool,
    /// Whether the microphone can be muted during calls
    pub can_mute_microphone: bool,
    /// Whether the speaker output can be muted during calls
    pub can_mute_speaker: bool,
    /// Whether DTMF tones can be sent during calls
    pub can_send_dtmf: bool,
    /// Whether calls can be transferred to other parties
    pub can_transfer: bool,
    /// Whether SDP offer/answer negotiation is supported
    pub supports_sdp_offer_answer: bool,
    /// Whether RTP (Real-time Transport Protocol) is supported
    pub supports_rtp: bool,
    /// Whether RTCP (RTP Control Protocol) is supported
    pub supports_rtcp: bool,
    /// Maximum number of concurrent calls supported
    pub max_concurrent_calls: usize,
    /// Supported media types (e.g., "audio", "video")
    pub supported_media_types: Vec<String>, // "audio", "video"
}

/// Capabilities available for a specific call in its current state
/// 
/// Unlike MediaCapabilities which describes global client capabilities,
/// this represents what operations are currently possible for a specific call.
#[derive(Debug, Clone)]
pub struct CallCapabilities {
    /// Whether this call can be placed on hold
    pub can_hold: bool,
    /// Whether this call can be resumed from hold
    pub can_resume: bool,
    /// Whether this call can be transferred
    pub can_transfer: bool,
    /// Whether DTMF tones can be sent for this call
    pub can_send_dtmf: bool,
    /// Whether the microphone can be muted for this call
    pub can_mute: bool,
    /// Whether this call can be terminated (hung up)
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
/// 
/// Represents the current state of media transmission for a call,
/// including session identifiers, ports, and quality information.
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    /// Call ID that this media session belongs to
    pub call_id: CallId,
    /// Session-core session identifier
    pub session_id: rvoip_session_core::api::types::SessionId,
    /// Media session identifier string
    pub media_session_id: String,
    /// Local RTP port for this media session
    pub local_rtp_port: Option<u16>,
    /// Remote RTP port for this media session
    pub remote_rtp_port: Option<u16>,
    /// Currently active codec for this session
    pub codec: Option<String>,
    /// Audio direction for this media session
    pub media_direction: AudioDirection,
    /// Current quality metrics (if available)
    pub quality_metrics: Option<AudioQualityMetrics>,
    /// Whether the media session is currently active
    pub is_active: bool,
    /// When this media session was created
    pub created_at: DateTime<Utc>,
}

/// Parameters negotiated between call endpoints
/// 
/// Contains the final agreed-upon media parameters after SDP negotiation,
/// representing the actual configuration being used for the call.
#[derive(Debug, Clone)]
pub struct NegotiatedMediaParams {
    /// Call ID these parameters apply to
    pub call_id: CallId,
    /// Final negotiated codec name
    pub negotiated_codec: Option<String>,
    /// Local RTP port being used
    pub local_rtp_port: Option<u16>,
    /// Remote RTP port being used
    pub remote_rtp_port: Option<u16>,
    /// Negotiated audio direction
    pub audio_direction: AudioDirection,
    /// Local SDP that was sent
    pub local_sdp: String,
    /// Remote SDP that was received
    pub remote_sdp: String,
    /// When the negotiation was completed
    pub negotiated_at: DateTime<Utc>,
    /// Whether DTMF is supported in this negotiation
    pub supports_dtmf: bool,
    /// Whether hold/resume is supported in this negotiation
    pub supports_hold: bool,
    /// Negotiated bandwidth limit in kilobits per second
    pub bandwidth_kbps: Option<u32>,
    /// Whether media encryption is enabled
    pub encryption_enabled: bool,
}

/// Enhanced media capabilities including advanced features
/// 
/// Extends basic MediaCapabilities with advanced session management
/// and coordination features for sophisticated VoIP applications.
#[derive(Debug, Clone)]
pub struct EnhancedMediaCapabilities {
    /// Basic media capabilities (codecs, hold, mute, etc.)
    pub basic_capabilities: MediaCapabilities,
    /// Whether SDP offer/answer pattern is supported
    pub supports_sdp_offer_answer: bool,
    /// Whether full media session lifecycle management is available
    pub supports_media_session_lifecycle: bool,
    /// Whether SDP can be renegotiated during active calls
    pub supports_sdp_renegotiation: bool,
    /// Whether early media (before call answer) is supported
    pub supports_early_media: bool,
    /// Whether media sessions can be updated/modified during calls
    pub supports_media_session_updates: bool,
    /// Whether codec negotiation is supported
    pub supports_codec_negotiation: bool,
    /// Whether bandwidth management is available
    pub supports_bandwidth_management: bool,
    /// Whether media encryption/security is supported
    pub supports_encryption: bool,
    /// Supported SDP protocol version
    pub supported_sdp_version: String,
    /// Maximum number of simultaneous media sessions
    pub max_media_sessions: usize,
    /// Preferred range for RTP port allocation (start, end)
    pub preferred_rtp_port_range: (u16, u16),
    /// Supported transport protocols (e.g., "RTP/AVP", "RTP/SAVP")
    pub supported_transport_protocols: Vec<String>,
}
