//! Core types for session-core-v2
//!
//! This module defines the fundamental types used throughout the session-core-v2 crate.
//! It includes identifiers, events, and common data structures.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// Re-export types from api::types for backwards compatibility
pub use crate::api::types::{
    AudioFrame,
    AudioFrameSubscriber,
    AudioStreamConfig,
    CallDecision,
    CallDirection,
    CallSession,
    CallState,
    IncomingCall,
    MediaInfo,
    PreparedCall,
    Session,
    SessionRole,
    SessionStats,
    StatusCode,
    TerminationReason,
};

// Re-export the ID types from state_table::types for convenience
pub use crate::state_table::types::{SessionId, DialogId, MediaSessionId};

/// Session events that flow through the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    /// Incoming call received
    IncomingCall {
        from: String,
        to: String,
        call_id: String,
        dialog_id: DialogId,
        sdp: Option<String>,
    },
    /// Call progress update
    CallProgress {
        dialog_id: DialogId,
        status_code: u16,
        reason: Option<String>,
    },
    /// Call was answered
    CallAnswered {
        dialog_id: DialogId,
        sdp: Option<String>,
    },
    /// Call was terminated
    CallTerminated {
        dialog_id: DialogId,
        reason: Option<String>,
    },
    /// Call failed
    CallFailed {
        dialog_id: DialogId,
        reason: String,
    },
    /// Media state changed
    MediaStateChanged {
        media_id: MediaSessionId,
        state: MediaState,
    },
    /// DTMF digit received
    DtmfReceived {
        dialog_id: DialogId,
        digit: char,
    },
    /// Hold request
    HoldRequest {
        dialog_id: DialogId,
    },
    /// Resume request
    ResumeRequest {
        dialog_id: DialogId,
    },
    /// Transfer request
    TransferRequest {
        dialog_id: DialogId,
        target: String,
        attended: bool,
    },
}

/// Media session state
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaState {
    /// Media not initialized
    Idle,
    /// Negotiating media
    Negotiating,
    /// Media is active
    Active,
    /// Media is on hold
    OnHold,
    /// Media failed
    Failed(String),
    /// Media session ended
    Terminated,
}

/// Transfer status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Transfer initiated
    Initiated,
    /// Transfer in progress
    InProgress,
    /// Transfer completed
    Completed,
    /// Transfer failed
    Failed(String),
}

/// Media direction for hold/resume
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaDirection {
    /// Send and receive media
    SendRecv,
    /// Send only (remote on hold)
    SendOnly,
    /// Receive only (local on hold)
    RecvOnly,
    /// No media flow (both on hold)
    Inactive,
}

/// Registration state
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationState {
    /// Not registered
    Unregistered,
    /// Registration in progress
    Registering,
    /// Successfully registered
    Registered,
    /// Registration failed
    Failed(String),
    /// Unregistering
    Unregistering,
}

/// Presence status types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    /// Available
    Available,
    /// Away
    Away,
    /// Busy
    Busy,
    /// Do not disturb
    DoNotDisturb,
    /// Offline
    Offline,
    /// Custom status
    Custom(String),
}

/// User credentials for authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// Username/account
    pub username: String,
    /// Password
    pub password: String,
    /// Optional realm
    pub realm: Option<String>,
}

impl Credentials {
    /// Create new credentials
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            realm: None,
        }
    }

    /// Set the realm
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
    }
}

/// Audio device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    /// Device ID
    pub id: String,
    /// Device name
    pub name: String,
    /// Is input device
    pub is_input: bool,
    /// Is output device
    pub is_output: bool,
    /// Sample rates supported
    pub sample_rates: Vec<u32>,
    /// Number of channels
    pub channels: u8,
}

/// Conference identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConferenceId(pub Uuid);

impl ConferenceId {
    /// Create a new conference ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConferenceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConferenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Call detail record for billing/logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallDetailRecord {
    /// Session ID
    pub session_id: SessionId,
    /// Dialog ID
    pub dialog_id: DialogId,
    /// Caller
    pub from: String,
    /// Called party
    pub to: String,
    /// Start time
    pub start_time: std::time::SystemTime,
    /// End time
    pub end_time: Option<std::time::SystemTime>,
    /// Duration in seconds
    pub duration: Option<u64>,
    /// Termination reason
    pub termination_reason: Option<String>,
    /// SIP Call-ID
    pub call_id: String,
}

/// Information about an incoming call
#[derive(Debug, Clone)]
pub struct IncomingCallInfo {
    /// Session ID assigned to this call
    pub session_id: SessionId,
    /// Dialog ID for this call
    pub dialog_id: DialogId,
    /// Caller URI
    pub from: String,
    /// Called party URI
    pub to: String,
    /// SIP Call-ID
    pub call_id: String,
}