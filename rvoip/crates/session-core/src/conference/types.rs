//! Core Conference Types
//!
//! Defines the fundamental types used throughout the conference system.

use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::api::types::SessionId;

/// Unique identifier for a conference
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConferenceId(pub String);

impl ConferenceId {
    pub fn new() -> Self {
        let id = format!("conf_{}", Uuid::new_v4());
        Self(id)
    }

    pub fn from_name(name: &str) -> Self {
        Self(name.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConferenceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Configuration for a conference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConferenceConfig {
    /// Maximum number of participants allowed
    pub max_participants: usize,
    /// Whether to enable audio mixing
    pub audio_mixing_enabled: bool,
    /// Audio sample rate for mixing (Hz)
    pub audio_sample_rate: u32,
    /// Audio channels (1 = mono, 2 = stereo)
    pub audio_channels: u8,
    /// RTP port range for media sessions
    pub rtp_port_range: Option<(u16, u16)>,
    /// Conference timeout (None = no timeout)
    pub timeout: Option<Duration>,
    /// Conference name for SDP and logging
    pub name: String,
}

impl Default for ConferenceConfig {
    fn default() -> Self {
        Self {
            max_participants: 10,
            audio_mixing_enabled: true,
            audio_sample_rate: 8000, // 8kHz for telephony
            audio_channels: 1, // Mono
            rtp_port_range: Some((10000, 20000)),
            timeout: None,
            name: "Conference Room".to_string(),
        }
    }
}

/// Status of a participant in a conference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantStatus {
    /// Participant is joining (INVITE sent/received)
    Joining,
    /// Participant is active in the conference
    Active,
    /// Participant is on hold
    OnHold,
    /// Participant is muted
    Muted,
    /// Participant is leaving (BYE sent/received)
    Leaving,
    /// Participant has left the conference
    Left,
}

/// Current state of a conference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConferenceState {
    /// Conference is being created
    Creating,
    /// Conference is active and accepting participants
    Active,
    /// Conference is locked (no new participants)
    Locked,
    /// Conference is being terminated
    Terminating,
    /// Conference has ended
    Terminated,
}

/// Statistics for a conference
#[derive(Debug, Clone)]
pub struct ConferenceStats {
    /// Total number of participants
    pub total_participants: usize,
    /// Number of active participants
    pub active_participants: usize,
    /// Number of participants with active audio
    pub audio_participants: usize,
    /// Conference duration
    pub duration: Duration,
    /// Conference state
    pub state: ConferenceState,
    /// Whether audio mixing is enabled
    pub audio_mixing_enabled: bool,
    /// Time when conference was created
    pub created_at: Instant,
}

/// Information about a participant
#[derive(Debug, Clone)]
pub struct ParticipantInfo {
    /// Session ID for this participant
    pub session_id: SessionId,
    /// SIP URI of the participant
    pub sip_uri: String,
    /// Display name (if available)
    pub display_name: Option<String>,
    /// Current status in conference
    pub status: ParticipantStatus,
    /// RTP port for media (if established)
    pub rtp_port: Option<u16>,
    /// Whether participant has audio active
    pub audio_active: bool,
    /// When participant joined
    pub joined_at: Instant,
}

/// Media configuration for conference
#[derive(Debug, Clone)]
pub struct ConferenceMediaConfig {
    /// Enable audio mixing
    pub enable_mixing: bool,
    /// Audio sample rate
    pub sample_rate: u32,
    /// Audio channels
    pub channels: u8,
    /// Samples per frame
    pub samples_per_frame: usize,
    /// Enable AGC
    pub enable_agc: bool,
    /// Enable noise reduction
    pub enable_noise_reduction: bool,
}

impl Default for ConferenceMediaConfig {
    fn default() -> Self {
        Self {
            enable_mixing: true,
            sample_rate: 8000,
            channels: 1,
            samples_per_frame: 160, // 20ms at 8kHz
            enable_agc: false,
            enable_noise_reduction: false,
        }
    }
} 