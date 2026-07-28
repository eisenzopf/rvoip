//! Core types for rvoip-sip
//!
//! This module defines the fundamental types used throughout the rvoip-sip crate.
//! It includes identifiers, events, and common data structures.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// Re-export types from api::types for backwards compatibility
pub use crate::api::types::{
    AudioFrame, AudioStreamConfig, CallDecision, CallDirection, CallSession, IncomingCall,
    MediaInfo, PreparedCall, Session, SessionRole, SessionStats, StatusCode, TerminationReason,
};

// Re-export the ID types from state_table::types for convenience
pub use crate::state_table::types::{DialogId, MediaSessionId, SessionId};

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
    /// Local user requested cancel before a provisional response. RFC 3261
    /// forbids sending CANCEL until a provisional response has arrived.
    CancelPending,
    /// CANCEL has been sent, or a late 200 OK is being ACK/BYE cleaned up.
    Cancelling,
    Ringing,
    Answering, // UAS accepted call, sending 200 OK, waiting for ACK
    /// UAS sent 200 OK and local hangup was requested before ACK arrived.
    AnsweringHangupPending,
    EarlyMedia,
    Active,
    /// Sent a hold re-INVITE, awaiting 2xx (RFC 3261 §14.1). Session
    /// parameters remain as they were in `Active` until the peer confirms.
    HoldPending,
    OnHold,
    Resuming,
    Muted,
    Bridged, // Two endpoint calls bridged together
    Transferring,
    TransferringCall, // Transfer recipient processing transfer
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
    Authenticating, // Processing authentication challenge

    // Messaging
    Messaging, // Handling SIP MESSAGE requests
}

impl CallState {
    /// Check if this is a final state (call is over)
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            CallState::Terminated | CallState::Cancelled | CallState::Failed(_)
        )
    }

    /// Check if the call is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            CallState::Initiating
                | CallState::CancelPending
                | CallState::Cancelling
                | CallState::Ringing
                | CallState::Answering
                | CallState::AnsweringHangupPending
                | CallState::Active
                | CallState::HoldPending
                | CallState::OnHold
                | CallState::EarlyMedia
                | CallState::Resuming
                | CallState::Muted
                | CallState::Bridged
                | CallState::Transferring
                | CallState::TransferringCall
                | CallState::ConsultationCall
        )
    }
}

impl fmt::Display for CallState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallState::Idle => write!(f, "Idle"),
            CallState::Initiating => write!(f, "Initiating"),
            CallState::CancelPending => write!(f, "CancelPending"),
            CallState::Cancelling => write!(f, "Cancelling"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Answering => write!(f, "Answering"),
            CallState::AnsweringHangupPending => write!(f, "AnsweringHangupPending"),
            CallState::EarlyMedia => write!(f, "EarlyMedia"),
            CallState::Active => write!(f, "Active"),
            CallState::HoldPending => write!(f, "HoldPending"),
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
#[derive(Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub start_time: std::time::SystemTime,
    pub media_active: bool,
}

impl fmt::Debug for CallInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallInfo")
            .field("session_id", &self.session_id)
            .field("from_bytes", &self.from.len())
            .field("to_bytes", &self.to.len())
            .field("state", &self.state)
            .field("media_active", &self.media_active)
            .finish()
    }
}

/// Session information for queries
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub start_time: std::time::SystemTime,
    pub media_active: bool,
}

impl fmt::Debug for SessionInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionInfo")
            .field("session_id", &self.session_id)
            .field("from_bytes", &self.from.len())
            .field("to_bytes", &self.to.len())
            .field("state", &self.state)
            .field("media_active", &self.media_active)
            .finish()
    }
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
    pub fn try_recv(
        &mut self,
    ) -> Result<rvoip_media_core::types::AudioFrame, tokio::sync::mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

/// Session events that flow through the system
#[derive(Clone, Serialize, Deserialize)]
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
    CallFailed { dialog_id: DialogId, reason: String },
    /// Media state changed
    MediaStateChanged {
        media_id: MediaSessionId,
        state: MediaState,
    },
    /// DTMF digit received
    DtmfReceived { dialog_id: DialogId, digit: char },
    /// Hold request
    HoldRequest { dialog_id: DialogId },
    /// Resume request
    ResumeRequest { dialog_id: DialogId },
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
    UnregistrationComplete { dialog_id: DialogId },
    /// Subscription started
    SubscriptionStarted {
        dialog_id: DialogId,
        uri: String,
        event_package: String,
        expires: u32,
    },
    /// Subscription accepted
    SubscriptionAccepted { dialog_id: DialogId, expires: u32 },
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
    MessageDelivered { dialog_id: DialogId },
    /// MESSAGE delivery failed
    MessageDeliveryFailed {
        dialog_id: DialogId,
        reason: String,
        status_code: u16,
    },
}

impl fmt::Debug for SessionEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncomingCall { from, to, sdp, .. } => formatter
                .debug_struct("IncomingCall")
                .field("from_bytes", &from.len())
                .field("to_bytes", &to.len())
                .field("sdp_present", &sdp.is_some())
                .field("sdp_bytes", &sdp.as_ref().map_or(0, String::len))
                .finish(),
            Self::CallProgress {
                status_code,
                reason,
                ..
            } => formatter
                .debug_struct("CallProgress")
                .field("status_code", status_code)
                .field("reason_present", &reason.is_some())
                .field("reason_bytes", &reason.as_ref().map_or(0, String::len))
                .finish(),
            Self::CallAnswered { sdp, .. } => formatter
                .debug_struct("CallAnswered")
                .field("sdp_present", &sdp.is_some())
                .field("sdp_bytes", &sdp.as_ref().map_or(0, String::len))
                .finish(),
            Self::CallTerminated { reason, .. } => formatter
                .debug_struct("CallTerminated")
                .field("reason_present", &reason.is_some())
                .field("reason_bytes", &reason.as_ref().map_or(0, String::len))
                .finish(),
            Self::CallFailed { reason, .. } => formatter
                .debug_struct("CallFailed")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::MediaStateChanged { state, .. } => formatter
                .debug_struct("MediaStateChanged")
                .field("state", state)
                .finish(),
            Self::DtmfReceived { .. } => formatter.write_str("DtmfReceived"),
            Self::HoldRequest { .. } => formatter.write_str("HoldRequest"),
            Self::ResumeRequest { .. } => formatter.write_str("ResumeRequest"),
            Self::TransferRequest {
                attended, target, ..
            } => formatter
                .debug_struct("TransferRequest")
                .field("target_bytes", &target.len())
                .field("attended", attended)
                .finish(),
            Self::RegistrationStarted { uri, expires, .. } => formatter
                .debug_struct("RegistrationStarted")
                .field("uri_bytes", &uri.len())
                .field("expires", expires)
                .finish(),
            Self::RegistrationSuccess { uri, expires, .. } => formatter
                .debug_struct("RegistrationSuccess")
                .field("uri_bytes", &uri.len())
                .field("expires", expires)
                .finish(),
            Self::RegistrationFailed {
                reason,
                status_code,
                ..
            } => formatter
                .debug_struct("RegistrationFailed")
                .field("reason_bytes", &reason.len())
                .field("status_code", status_code)
                .finish(),
            Self::UnregistrationComplete { .. } => formatter.write_str("UnregistrationComplete"),
            Self::SubscriptionStarted {
                uri,
                event_package,
                expires,
                ..
            } => formatter
                .debug_struct("SubscriptionStarted")
                .field("uri_bytes", &uri.len())
                .field("event_package_bytes", &event_package.len())
                .field("expires", expires)
                .finish(),
            Self::SubscriptionAccepted { expires, .. } => formatter
                .debug_struct("SubscriptionAccepted")
                .field("expires", expires)
                .finish(),
            Self::SubscriptionFailed {
                reason,
                status_code,
                ..
            } => formatter
                .debug_struct("SubscriptionFailed")
                .field("reason_bytes", &reason.len())
                .field("status_code", status_code)
                .finish(),
            Self::NotifyReceived {
                event_package,
                body,
                ..
            } => formatter
                .debug_struct("NotifyReceived")
                .field("event_package_bytes", &event_package.len())
                .field("body_present", &body.is_some())
                .field("body_bytes", &body.as_ref().map_or(0, String::len))
                .finish(),
            Self::MessageSent { to, body, .. } => formatter
                .debug_struct("MessageSent")
                .field("to_bytes", &to.len())
                .field("body_bytes", &body.len())
                .finish(),
            Self::MessageReceived { from, body, .. } => formatter
                .debug_struct("MessageReceived")
                .field("from_bytes", &from.len())
                .field("body_bytes", &body.len())
                .finish(),
            Self::MessageDelivered { .. } => formatter.write_str("MessageDelivered"),
            Self::MessageDeliveryFailed {
                reason,
                status_code,
                ..
            } => formatter
                .debug_struct("MessageDeliveryFailed")
                .field("reason_bytes", &reason.len())
                .field("status_code", status_code)
                .finish(),
        }
    }
}

/// Media session state
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Debug for MediaState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Negotiating => formatter.write_str("Negotiating"),
            Self::Active => formatter.write_str("Active"),
            Self::OnHold => formatter.write_str("OnHold"),
            Self::Failed(reason) => formatter
                .debug_struct("Failed")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Terminated => formatter.write_str("Terminated"),
        }
    }
}

/// Transfer status
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Debug for TransferStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initiated => formatter.write_str("Initiated"),
            Self::InProgress => formatter.write_str("InProgress"),
            Self::Completed => formatter.write_str("Completed"),
            Self::Failed(reason) => formatter
                .debug_struct("Failed")
                .field("reason_bytes", &reason.len())
                .finish(),
        }
    }
}

/// Media direction for hold/resume
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Debug for RegistrationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unregistered => formatter.write_str("Unregistered"),
            Self::Registering => formatter.write_str("Registering"),
            Self::Registered => formatter.write_str("Registered"),
            Self::Failed(reason) => formatter
                .debug_struct("Failed")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Unregistering => formatter.write_str("Unregistering"),
        }
    }
}

/// Presence status types
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Debug for PresenceStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available => formatter.write_str("Available"),
            Self::Away => formatter.write_str("Away"),
            Self::Busy => formatter.write_str("Busy"),
            Self::DoNotDisturb => formatter.write_str("DoNotDisturb"),
            Self::Offline => formatter.write_str("Offline"),
            Self::Custom(value) => formatter
                .debug_struct("Custom")
                .field("bytes", &value.len())
                .finish(),
        }
    }
}

/// User credentials for authentication
#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// Username/account
    pub username: String,
    /// Password
    pub password: String,
    /// Optional realm
    pub realm: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &"[redacted]")
            .field("password", &"[redacted]")
            .field("has_realm", &self.realm.is_some())
            .finish()
    }
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
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for AudioDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioDevice")
            .field("id_bytes", &self.id.len())
            .field("name_bytes", &self.name.len())
            .field("is_input", &self.is_input)
            .field("is_output", &self.is_output)
            .field("sample_rates", &self.sample_rates)
            .field("channels", &self.channels)
            .finish()
    }
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
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for CallDetailRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallDetailRecord")
            .field("session_id", &self.session_id)
            .field("dialog_id", &"[opaque]")
            .field("from_bytes", &self.from.len())
            .field("to_bytes", &self.to.len())
            .field("end_time_present", &self.end_time.is_some())
            .field("duration", &self.duration)
            .field(
                "termination_reason_present",
                &self.termination_reason.is_some(),
            )
            .field(
                "termination_reason_bytes",
                &self.termination_reason.as_ref().map_or(0, String::len),
            )
            .field("call_id_bytes", &self.call_id.len())
            .finish()
    }
}

/// Information about an incoming call
#[derive(Clone)]
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
    /// `P-Asserted-Identity` header value (RFC 3325 §9.1) when the inbound
    /// INVITE carried one — typical on calls coming from a carrier trunk
    /// or trusted PBX. The string is the wire form of the header value
    /// (e.g. `"\"Alice\" <sip:alice@example.com>, <tel:+14155551234>"`);
    /// callers wanting structured access can re-parse with
    /// `rvoip_sip_core::types::p_asserted_identity::PAssertedIdentity::from_str`.
    pub p_asserted_identity: Option<String>,
}

impl fmt::Debug for IncomingCallInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IncomingCallInfo")
            .field("session_id", &self.session_id)
            .field("dialog_id", &"[opaque]")
            .field("from_bytes", &self.from.len())
            .field("to_bytes", &self.to.len())
            .field("call_id_bytes", &self.call_id.len())
            .field(
                "p_asserted_identity_present",
                &self.p_asserted_identity.is_some(),
            )
            .field(
                "p_asserted_identity_bytes",
                &self.p_asserted_identity.as_ref().map_or(0, String::len),
            )
            .finish()
    }
}

#[cfg(test)]
mod diagnostic_safety_tests {
    use super::*;

    #[test]
    fn public_session_and_cdr_debug_is_payload_free() {
        const SECRET: &str = "legacy-session-type-secret-canary";
        let session_id = SessionId::from_string(SECRET);
        let dialog_id = DialogId::new();
        let event = SessionEvent::IncomingCall {
            from: SECRET.to_string(),
            to: SECRET.to_string(),
            call_id: SECRET.to_string(),
            dialog_id,
            sdp: Some(SECRET.to_string()),
        };
        let cdr = CallDetailRecord {
            session_id: session_id.clone(),
            dialog_id,
            from: SECRET.to_string(),
            to: SECRET.to_string(),
            start_time: std::time::SystemTime::now(),
            end_time: None,
            duration: None,
            termination_reason: Some(SECRET.to_string()),
            call_id: SECRET.to_string(),
        };
        let incoming = IncomingCallInfo {
            session_id,
            dialog_id,
            from: SECRET.to_string(),
            to: SECRET.to_string(),
            call_id: SECRET.to_string(),
            p_asserted_identity: Some(SECRET.to_string()),
        };

        for rendered in [
            format!("{event:?}"),
            format!("{cdr:?}"),
            format!("{incoming:?}"),
        ] {
            assert!(!rendered.contains(SECRET), "debug leaked: {rendered}");
        }
    }
}
