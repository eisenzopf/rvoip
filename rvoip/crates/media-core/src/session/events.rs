//! Media Session Events
//!
//! This module defines events that MediaSession can emit to notify other components
//! about codec changes, quality issues, and media session lifecycle events.

use crate::types::{MediaSessionId, MediaType, DialogId};
use crate::quality::metrics::QualityMetrics;
use std::time::Instant;

/// Media session event types
#[derive(Debug, Clone)]
pub enum MediaSessionEventType {
    /// Session was created successfully
    SessionCreated,
    
    /// Session was destroyed
    SessionDestroyed,
    
    /// Codec was changed (e.g., due to negotiation or adaptation)
    CodecChanged {
        media_type: MediaType,
        old_codec: String,
        new_codec: String,
    },
    
    /// Quality threshold exceeded
    QualityIssue {
        metrics: QualityMetrics,
        severity: QualitySeverity,
    },
    
    /// Media processing error occurred
    ProcessingError {
        error_type: String,
        details: String,
    },
    
    /// RTP packet statistics update
    PacketStats {
        packets_sent: u64,
        packets_received: u64,
        bytes_sent: u64,
        bytes_received: u64,
    },
}

/// Quality issue severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualitySeverity {
    /// Minor quality degradation
    Minor,
    /// Moderate quality issues
    Moderate,
    /// Severe quality problems
    Severe,
    /// Critical quality failure
    Critical,
}

/// Media session event
#[derive(Debug, Clone)]
pub struct MediaSessionEvent {
    /// Session that generated the event
    pub session_id: MediaSessionId,
    
    /// Associated dialog ID
    pub dialog_id: DialogId,
    
    /// Event type and payload
    pub event_type: MediaSessionEventType,
    
    /// Event timestamp
    pub timestamp: Instant,
}

impl MediaSessionEvent {
    /// Create a new media session event
    pub fn new(
        session_id: MediaSessionId,
        dialog_id: DialogId,
        event_type: MediaSessionEventType,
    ) -> Self {
        Self {
            session_id,
            dialog_id,
            event_type,
            timestamp: Instant::now(),
        }
    }
    
    /// Create a session created event
    pub fn session_created(session_id: MediaSessionId, dialog_id: DialogId) -> Self {
        Self::new(session_id, dialog_id, MediaSessionEventType::SessionCreated)
    }
    
    /// Create a session destroyed event
    pub fn session_destroyed(session_id: MediaSessionId, dialog_id: DialogId) -> Self {
        Self::new(session_id, dialog_id, MediaSessionEventType::SessionDestroyed)
    }
    
    /// Create a codec changed event
    pub fn codec_changed(
        session_id: MediaSessionId,
        dialog_id: DialogId,
        media_type: MediaType,
        old_codec: String,
        new_codec: String,
    ) -> Self {
        Self::new(
            session_id,
            dialog_id,
            MediaSessionEventType::CodecChanged {
                media_type,
                old_codec,
                new_codec,
            },
        )
    }
    
    /// Create a quality issue event
    pub fn quality_issue(
        session_id: MediaSessionId,
        dialog_id: DialogId,
        metrics: QualityMetrics,
        severity: QualitySeverity,
    ) -> Self {
        Self::new(
            session_id,
            dialog_id,
            MediaSessionEventType::QualityIssue { metrics, severity },
        )
    }
}

impl QualitySeverity {
    /// Determine quality severity from MOS score
    pub fn from_mos_score(mos: f32) -> Self {
        match mos {
            mos if mos >= 3.5 => Self::Minor,
            mos if mos >= 2.5 => Self::Moderate,
            mos if mos >= 1.5 => Self::Severe,
            _ => Self::Critical,
        }
    }
    
    /// Get severity level as number (higher = more severe)
    pub fn level(&self) -> u8 {
        match self {
            Self::Minor => 1,
            Self::Moderate => 2,
            Self::Severe => 3,
            Self::Critical => 4,
        }
    }
} 