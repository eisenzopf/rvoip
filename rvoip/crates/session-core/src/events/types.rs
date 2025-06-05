//! Event Types
//!
//! Types for session events.

use crate::api::types::{SessionId, CallSession, CallState};

/// Session event types
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Session was created
    Created {
        session_id: SessionId,
        from: String,
        to: String,
    },

    /// Session state changed
    StateChanged {
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    },

    /// Session terminated
    Terminated {
        session_id: SessionId,
        reason: String,
    },

    /// Media event
    Media {
        session_id: SessionId,
        event_type: MediaEventType,
    },

    /// Error occurred
    Error {
        session_id: Option<SessionId>,
        message: String,
    },
}

/// Media event types
#[derive(Debug, Clone)]
pub enum MediaEventType {
    StreamStarted,
    StreamStopped,
    CodecChanged(String),
    PortChanged(u16),
} 