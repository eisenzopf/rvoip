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
    AudioStreamConfig,
    CallDecision,
    CallDirection,
    CallSession,
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

/// Reasons for call failure
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum FailureReason {
    Timeout,
    Rejected,
    NetworkError,
    MediaError,
    ProtocolError,
    Other,
}

impl fmt::Display for FailureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FailureReason::Timeout => write!(f, "Timeout"),
            FailureReason::Rejected => write!(f, "Rejected"),
            FailureReason::NetworkError => write!(f, "Network error"),
            FailureReason::MediaError => write!(f, "Media error"),
            FailureReason::ProtocolError => write!(f, "Protocol error"),
            FailureReason::Other => write!(f, "Other error"),
        }
    }
}

/// Call states
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallState {
    Idle,
    Initiating,
    Ringing,
    Answering,        // UAS accepted call, sending 200 OK, waiting for ACK
    EarlyMedia,
    Active,
    OnHold,
    Resuming,
    Muted,
    Bridged,           // Two endpoint calls bridged together
    Transferring,
    TransferringCall,  // Transfer recipient processing transfer
    ConsultationCall,
    Terminating,
    Terminated,
    Cancelled,
    Failed(FailureReason),
    
    // Registration states
    Registering,
    Registered,
    Unregistering,
    
    // Subscription/Presence states
    Subscribing,
    Subscribed,
    Publishing,
    
    
    // Authentication flow
    Authenticating,     // Processing authentication challenge
    
    
    // Messaging
    Messaging,         // Handling SIP MESSAGE requests
    
}

impl CallState {
    /// Check if this is a final state (call is over)
    pub fn is_final(&self) -> bool {
        matches!(self, CallState::Terminated | CallState::Cancelled | CallState::Failed(_))
    }

    /// Check if the call is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(self, 
            CallState::Initiating | 
            CallState::Ringing | 
            CallState::Active | 
            CallState::OnHold |
            CallState::EarlyMedia |
            CallState::Resuming |
            CallState::Muted |
            CallState::Bridged |
            CallState::Transferring |
            CallState::TransferringCall |
            CallState::ConsultationCall
        )
    }
}

impl fmt::Display for CallState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallState::Idle => write!(f, "Idle"),
            CallState::Initiating => write!(f, "Initiating"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Answering => write!(f, "Answering"),
            CallState::EarlyMedia => write!(f, "EarlyMedia"),
            CallState::Active => write!(f, "Active"),
            CallState::OnHold => write!(f, "OnHold"),
            CallState::Resuming => write!(f, "Resuming"),
            CallState::Muted => write!(f, "Muted"),
            CallState::Bridged => write!(f, "Bridged"),
            CallState::Transferring => write!(f, "Transferring"),
            CallState::TransferringCall => write!(f, "TransferringCall"),
            CallState::ConsultationCall => write!(f, "ConsultationCall"),
            CallState::Terminating => write!(f, "Terminating"),
            CallState::Terminated => write!(f, "Terminated"),
            CallState::Cancelled => write!(f, "Cancelled"),
            CallState::Failed(reason) => write!(f, "Failed({})", reason),
            
            // Registration states
            CallState::Registering => write!(f, "Registering"),
            CallState::Registered => write!(f, "Registered"),
            CallState::Unregistering => write!(f, "Unregistering"),
            
            // Subscription/Presence states
            CallState::Subscribing => write!(f, "Subscribing"),
            CallState::Subscribed => write!(f, "Subscribed"),
            CallState::Publishing => write!(f, "Publishing"),
            
            // Authentication and routing states
            CallState::Authenticating => write!(f, "Authenticating"),
            CallState::Messaging => write!(f, "Messaging"),
        }
    }
}

/// Call information for active calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub start_time: std::time::SystemTime,
    pub media_active: bool,
}

/// Session information for queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub start_time: std::time::SystemTime,
    pub media_active: bool,
}

/// Audio frame subscriber for receiving decoded audio frames
pub struct AudioFrameSubscriber {
    /// Session ID this subscriber is associated with
    pub session_id: SessionId,
    /// Receiver for audio frames
    pub receiver: tokio::sync::mpsc::Receiver<rvoip_media_core::types::AudioFrame>,
}

impl AudioFrameSubscriber {
    /// Create a new audio frame subscriber
    pub fn new(
        session_id: SessionId,
        receiver: tokio::sync::mpsc::Receiver<rvoip_media_core::types::AudioFrame>,
    ) -> Self {
        Self {
            session_id,
            receiver,
        }
    }
    
    /// Receive the next audio frame (async)
    pub async fn recv(&mut self) -> Option<rvoip_media_core::types::AudioFrame> {
        self.receiver.recv().await
    }
    
    /// Try to receive an audio frame (non-blocking)
    pub fn try_recv(&mut self) -> Result<rvoip_media_core::types::AudioFrame, tokio::sync::mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

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
    /// Registration started
    RegistrationStarted {
        dialog_id: DialogId,
        uri: String,
        expires: u32,
    },
    /// Registration successful
    RegistrationSuccess {
        dialog_id: DialogId,
        uri: String,
        expires: u32,
    },
    /// Registration failed
    RegistrationFailed {
        dialog_id: DialogId,
        reason: String,
        status_code: u16,
    },
    /// Unregistration complete
    UnregistrationComplete {
        dialog_id: DialogId,
    },
    /// Subscription started
    SubscriptionStarted {
        dialog_id: DialogId,
        uri: String,
        event_package: String,
        expires: u32,
    },
    /// Subscription accepted
    SubscriptionAccepted {
        dialog_id: DialogId,
        expires: u32,
    },
    /// Subscription failed
    SubscriptionFailed {
        dialog_id: DialogId,
        reason: String,
        status_code: u16,
    },
    /// NOTIFY received
    NotifyReceived {
        dialog_id: DialogId,
        event_package: String,
        body: Option<String>,
    },
    /// MESSAGE sent
    MessageSent {
        dialog_id: DialogId,
        to: String,
        body: String,
    },
    /// MESSAGE received
    MessageReceived {
        dialog_id: DialogId,
        from: String,
        body: String,
    },
    /// MESSAGE delivery confirmed
    MessageDelivered {
        dialog_id: DialogId,
    },
    /// MESSAGE delivery failed
    MessageDeliveryFailed {
        dialog_id: DialogId,
        reason: String,
        status_code: u16,
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