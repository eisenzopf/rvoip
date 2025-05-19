//! Media events module
//!
//! This module defines events that can occur during media session operation.

use std::time::SystemTime;

use super::{MediaSessionId, MediaState, MediaDirection};

/// Media event type
#[derive(Debug, Clone)]
pub enum MediaEventType {
    /// Session has been created
    Created,
    
    /// Session state has changed
    StateChanged {
        old_state: MediaState,
        new_state: MediaState,
    },
    
    /// Session direction has changed
    DirectionChanged {
        old_direction: MediaDirection,
        new_direction: MediaDirection,
    },
    
    /// Session has been started
    Started,
    
    /// Session has been stopped
    Stopped,
    
    /// Session has been put on hold
    Held,
    
    /// Session has been resumed from hold
    Resumed,
    
    /// Session has encountered an error
    Error {
        /// Error message
        message: String,
        
        /// Error code
        code: u32,
    },
    
    /// Media quality has changed
    QualityChanged {
        /// New MOS score (1.0-5.0)
        mos: f32,
        
        /// New round-trip time in milliseconds
        rtt_ms: f64,
        
        /// New jitter in milliseconds
        jitter_ms: f64,
        
        /// New packet loss percentage
        packet_loss_percent: f32,
    },
    
    /// DTMF digit received
    DtmfReceived {
        /// DTMF digit (0-9, *, #, A-D)
        digit: char,
        
        /// Duration in milliseconds
        duration_ms: u32,
    },
    
    /// Remote endpoint has changed
    RemoteChanged {
        /// New remote address/description
        remote: String,
    },
    
    /// Custom event with arbitrary data
    Custom {
        /// Event name
        name: String,
        
        /// Event data as JSON-encoded string
        data: String,
    },
}

/// Media event with metadata
#[derive(Debug, Clone)]
pub struct MediaEvent {
    /// Session ID for this event
    pub session_id: MediaSessionId,
    
    /// Timestamp when the event occurred
    pub timestamp: SystemTime,
    
    /// Type of event
    pub event_type: MediaEventType,
}

impl MediaEvent {
    /// Create a new media event
    pub fn new(session_id: MediaSessionId, event_type: MediaEventType) -> Self {
        Self {
            session_id,
            timestamp: SystemTime::now(),
            event_type,
        }
    }
} 