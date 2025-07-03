//! Integration Events
//!
//! This module defines events for cross-crate communication between media-core
//! and other components like session-core and rtp-core.

use crate::types::{MediaSessionId, DialogId};
use crate::quality::metrics::QualityMetrics;
use std::time::Instant;

/// Integration event types for cross-crate communication
#[derive(Debug, Clone)]
pub enum IntegrationEventType {
    /// Media session was created and is ready
    MediaSessionReady {
        session_id: MediaSessionId,
        capabilities: MediaCapabilities,
    },
    
    /// Media session was destroyed
    MediaSessionDestroyed {
        session_id: MediaSessionId,
    },
    
    /// Quality metrics update
    QualityUpdate {
        session_id: MediaSessionId,
        metrics: QualityMetrics,
    },
    
    /// Codec negotiation request from session-core
    CodecNegotiationRequest {
        dialog_id: DialogId,
        offered_codecs: Vec<String>,
    },
    
    /// Codec negotiation response to session-core
    CodecNegotiationResponse {
        dialog_id: DialogId,
        selected_codec: String,
        parameters: CodecParameters,
    },
    
    /// RTP session registration request
    RtpSessionRegister {
        session_id: MediaSessionId,
        rtp_params: RtpParameters,
    },
    
    /// RTP session unregistration request
    RtpSessionUnregister {
        session_id: MediaSessionId,
    },
    
    /// Media packet received from RTP
    MediaPacketReceived {
        session_id: MediaSessionId,
        packet_info: PacketInfo,
    },
    
    /// Media packet ready to send via RTP
    MediaPacketSend {
        session_id: MediaSessionId,
        encoded_data: Vec<u8>,
        timestamp: u32,
    },
}

/// Media capabilities for SDP negotiation
#[derive(Debug, Clone)]
pub struct MediaCapabilities {
    /// Supported audio codecs
    pub audio_codecs: Vec<AudioCodecCapability>,
    /// Supported video codecs (future)
    pub video_codecs: Vec<VideoCodecCapability>,
    /// Audio processing capabilities
    pub audio_processing: AudioProcessingCapabilities,
}

/// Audio codec capability description
#[derive(Debug, Clone)]
pub struct AudioCodecCapability {
    /// Codec name (e.g., "PCMU", "PCMA", "Opus")
    pub name: String,
    /// Payload type
    pub payload_type: u8,
    /// Supported sample rates
    pub sample_rates: Vec<u32>,
    /// Supported channels
    pub channels: Vec<u8>,
    /// Codec-specific parameters
    pub parameters: CodecParameters,
}

/// Video codec capability (placeholder for future)
#[derive(Debug, Clone)]
pub struct VideoCodecCapability {
    /// Codec name
    pub name: String,
    /// Payload type
    pub payload_type: u8,
}

/// Audio processing capabilities
#[derive(Debug, Clone)]
pub struct AudioProcessingCapabilities {
    /// Echo cancellation available
    pub echo_cancellation: bool,
    /// Automatic gain control available
    pub automatic_gain_control: bool,
    /// Voice activity detection available
    pub voice_activity_detection: bool,
    /// Noise suppression available
    pub noise_suppression: bool,
}

/// Codec parameters for negotiation
#[derive(Debug, Clone, Default)]
pub struct CodecParameters {
    /// Bitrate
    pub bitrate: Option<u32>,
    /// Frame size in milliseconds
    pub frame_size_ms: Option<f32>,
    /// Variable bitrate enabled
    pub vbr: Option<bool>,
    /// Custom parameters
    pub custom: std::collections::HashMap<String, String>,
}

/// RTP session parameters
#[derive(Debug, Clone)]
pub struct RtpParameters {
    /// Local RTP port
    pub local_port: u16,
    /// Remote RTP address
    pub remote_address: String,
    /// Remote RTP port
    pub remote_port: u16,
    /// Payload type for this session
    pub payload_type: u8,
    /// SSRC for this session
    pub ssrc: u32,
}

/// Packet information for RTP integration
#[derive(Debug, Clone)]
pub struct PacketInfo {
    /// Payload type
    pub payload_type: u8,
    /// Sequence number
    pub sequence_number: u16,
    /// RTP timestamp
    pub timestamp: u32,
    /// SSRC
    pub ssrc: u32,
    /// Packet size
    pub size: usize,
}

/// Integration event for cross-crate communication
#[derive(Debug, Clone)]
pub struct IntegrationEvent {
    /// Event type and payload
    pub event_type: IntegrationEventType,
    /// Event timestamp
    pub timestamp: Instant,
    /// Source component
    pub source: String,
    /// Target component
    pub target: String,
}

impl IntegrationEvent {
    /// Create a new integration event
    pub fn new(
        event_type: IntegrationEventType,
        source: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            event_type,
            timestamp: Instant::now(),
            source: source.into(),
            target: target.into(),
        }
    }
    
    /// Create a media session ready event
    pub fn media_session_ready(
        session_id: MediaSessionId,
        capabilities: MediaCapabilities,
    ) -> Self {
        Self::new(
            IntegrationEventType::MediaSessionReady { session_id, capabilities },
            "media-core",
            "session-core",
        )
    }
    
    /// Create a codec negotiation request event
    pub fn codec_negotiation_request(
        dialog_id: DialogId,
        offered_codecs: Vec<String>,
    ) -> Self {
        Self::new(
            IntegrationEventType::CodecNegotiationRequest { dialog_id, offered_codecs },
            "session-core",
            "media-core",
        )
    }
    
    /// Create an RTP session register event
    pub fn rtp_session_register(
        session_id: MediaSessionId,
        rtp_params: RtpParameters,
    ) -> Self {
        Self::new(
            IntegrationEventType::RtpSessionRegister { session_id, rtp_params },
            "media-core",
            "rtp-core",
        )
    }
}

impl Default for MediaCapabilities {
    fn default() -> Self {
        Self {
            audio_codecs: vec![
                AudioCodecCapability {
                    name: "PCMU".to_string(),
                    payload_type: 0,
                    sample_rates: vec![8000],
                    channels: vec![1],
                    parameters: CodecParameters::default(),
                },
                AudioCodecCapability {
                    name: "PCMA".to_string(),
                    payload_type: 8,
                    sample_rates: vec![8000],
                    channels: vec![1],
                    parameters: CodecParameters::default(),
                },
            ],
            video_codecs: vec![],
            audio_processing: AudioProcessingCapabilities {
                echo_cancellation: true,
                automatic_gain_control: true,
                voice_activity_detection: true,
                noise_suppression: false, // Not implemented yet
            },
        }
    }
} 