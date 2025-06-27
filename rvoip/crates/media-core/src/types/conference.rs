//! Conference-related types for multi-party audio mixing
//!
//! This module defines types specific to conference audio mixing,
//! including participant management, mixing configuration, and
//! conference-specific events.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use super::{AudioFrame, MediaSessionId};
use crate::error::Result;

/// Unique identifier for a conference participant
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParticipantId(pub String);

impl ParticipantId {
    /// Create a new participant ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Get the string representation
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ParticipantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Audio stream for a conference participant
#[derive(Debug, Clone)]
pub struct AudioStream {
    /// Participant identifier
    pub participant_id: ParticipantId,
    
    /// Current audio format information
    pub sample_rate: u32,
    pub channels: u8,
    pub samples_per_frame: u32,
    
    /// Stream health and timing
    pub creation_time: Instant,
    pub last_frame_time: Option<Instant>,
    pub frames_received: u64,
    pub frames_dropped: u64,
    
    /// Audio processing state
    pub is_muted: bool,
    pub gain_level: f32,
    pub is_talking: bool, // Voice activity detection result
}

impl AudioStream {
    /// Create a new audio stream for a participant
    pub fn new(participant_id: ParticipantId, sample_rate: u32, channels: u8) -> Self {
        Self {
            participant_id,
            sample_rate,
            channels,
            samples_per_frame: (sample_rate / 50) as u32, // 20ms frames by default
            creation_time: Instant::now(),
            last_frame_time: None,
            frames_received: 0,
            frames_dropped: 0,
            is_muted: false,
            gain_level: 1.0,
            is_talking: false,
        }
    }
    
    /// Update stream statistics when a frame is received
    pub fn update_frame_received(&mut self) {
        self.last_frame_time = Some(Instant::now());
        self.frames_received += 1;
    }
    
    /// Update stream statistics when a frame is dropped
    pub fn update_frame_dropped(&mut self) {
        self.frames_dropped += 1;
    }
    
    /// Check if the stream is healthy (recently active)
    /// New streams are considered healthy for a grace period before health checks apply
    pub fn is_healthy(&self, timeout: Duration) -> bool {
        // Grace period for newly created streams (30 seconds)
        const GRACE_PERIOD: Duration = Duration::from_secs(30);
        
        // If stream was created recently, consider it healthy regardless of activity
        if self.creation_time.elapsed() < GRACE_PERIOD {
            return true;
        }
        
        // After grace period, check for recent activity
        if let Some(last_time) = self.last_frame_time {
            last_time.elapsed() < timeout
        } else {
            // No activity after grace period - not healthy
            false
        }
    }
    
    /// Check if the participant should be considered as talking
    /// New streams are considered talking during grace period, then VAD takes over
    pub fn is_effectively_talking(&self) -> bool {
        // Grace period for newly created streams (30 seconds)
        const GRACE_PERIOD: Duration = Duration::from_secs(30);
        
        // If stream was created recently, consider it talking regardless of VAD
        if self.creation_time.elapsed() < GRACE_PERIOD {
            return true;
        }
        
        // After grace period, use actual VAD result
        self.is_talking
    }
    
    /// Get the packet loss rate for this stream
    pub fn packet_loss_rate(&self) -> f32 {
        let total_frames = self.frames_received + self.frames_dropped;
        if total_frames > 0 {
            self.frames_dropped as f32 / total_frames as f32
        } else {
            0.0
        }
    }
}

/// Configuration for conference audio mixing
#[derive(Debug, Clone)]
pub struct ConferenceMixingConfig {
    /// Maximum number of participants in the conference
    pub max_participants: usize,
    
    /// Audio format for the mixed output
    pub output_sample_rate: u32,
    pub output_channels: u8,
    pub output_samples_per_frame: u32,
    
    /// Mixing behavior settings
    pub enable_voice_activity_mixing: bool, // Only mix talking participants
    pub enable_automatic_gain_control: bool,
    pub enable_noise_reduction: bool,
    
    /// Performance settings
    pub enable_simd_optimization: bool,
    pub max_concurrent_mixes: usize,
    
    /// Quality settings
    pub mixing_quality: MixingQuality,
    pub overflow_protection: bool,
}

impl Default for ConferenceMixingConfig {
    fn default() -> Self {
        Self {
            max_participants: 10,
            output_sample_rate: 8000, // 8kHz for telephony
            output_channels: 1, // Mono
            output_samples_per_frame: 160, // 20ms at 8kHz
            enable_voice_activity_mixing: true,
            enable_automatic_gain_control: true,
            enable_noise_reduction: false, // Can be expensive
            enable_simd_optimization: true,
            max_concurrent_mixes: 5,
            mixing_quality: MixingQuality::Balanced,
            overflow_protection: true,
        }
    }
}

/// Quality levels for audio mixing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixingQuality {
    /// Fast mixing with basic algorithms
    Fast,
    /// Balanced quality and performance
    Balanced,
    /// High quality mixing with advanced algorithms
    High,
}

/// Mixed audio output for a specific participant
#[derive(Debug, Clone)]
pub struct MixedAudioOutput {
    /// Target participant (who will receive this mix)
    pub target_participant: ParticipantId,
    
    /// Mixed audio frame (everyone except target participant)
    pub mixed_frame: Arc<AudioFrame>,
    
    /// Participants included in this mix
    pub included_participants: Vec<ParticipantId>,
    
    /// Mixing statistics
    pub mix_level: f32, // Overall volume level
    pub dominant_speaker: Option<ParticipantId>, // Loudest participant
    pub participant_count: usize,
}

/// Conference audio mixing statistics
#[derive(Debug, Clone, Default)]
pub struct ConferenceMixingStats {
    /// Total number of mixing operations performed
    pub total_mixes: u64,
    
    /// Number of participants currently active
    pub active_participants: usize,
    
    /// Average mixing latency in microseconds
    pub avg_mixing_latency_us: u64,
    
    /// CPU usage for mixing operations (0.0 to 1.0)
    pub cpu_usage: f32,
    
    /// Memory usage for conference buffers in bytes
    pub memory_usage_bytes: usize,
    
    /// Quality metrics
    pub overall_quality_score: f32, // 0.0 to 5.0 (MOS-like score)
    pub packet_loss_rate: f32,
    pub jitter_ms: f32,
}

/// Error types specific to conference audio mixing
#[derive(Debug, thiserror::Error)]
pub enum ConferenceError {
    #[error("Conference is at maximum capacity ({max} participants)")]
    ConferenceAtCapacity { max: usize },
    
    #[error("Participant {participant_id} not found in conference")]
    ParticipantNotFound { participant_id: ParticipantId },
    
    #[error("Participant {participant_id} already exists in conference")]
    ParticipantAlreadyExists { participant_id: ParticipantId },
    
    #[error("Audio format mismatch: expected {expected_sample_rate}Hz/{expected_channels}ch, got {actual_sample_rate}Hz/{actual_channels}ch")]
    AudioFormatMismatch {
        expected_sample_rate: u32,
        expected_channels: u8,
        actual_sample_rate: u32,
        actual_channels: u8,
    },
    
    #[error("Mixing operation failed: {reason}")]
    MixingFailed { reason: String },
    
    #[error("Audio frame processing error: {reason}")]
    FrameProcessingError { reason: String },
}

/// Conference mixing event for monitoring and coordination
#[derive(Debug, Clone)]
pub enum ConferenceMixingEvent {
    /// A participant was added to the conference
    ParticipantAdded {
        participant_id: ParticipantId,
        participant_count: usize,
    },
    
    /// A participant was removed from the conference
    ParticipantRemoved {
        participant_id: ParticipantId,
        participant_count: usize,
    },
    
    /// A participant started or stopped talking
    VoiceActivityChanged {
        participant_id: ParticipantId,
        is_talking: bool,
    },
    
    /// Conference audio quality changed significantly
    QualityChanged {
        old_score: f32,
        new_score: f32,
        reason: String,
    },
    
    /// Mixing performance warning
    PerformanceWarning {
        latency_us: u64,
        cpu_usage: f32,
        reason: String,
    },
}

/// Result type for conference operations
pub type ConferenceResult<T> = std::result::Result<T, ConferenceError>; 