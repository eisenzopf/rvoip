//! Cross-Crate Event Definitions
//!
//! Defines all events that cross crate boundaries, enabling event-driven
//! communication between session-core, dialog-core, media-core, etc.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::events::types::{Event, EventPriority};
use crate::planes::routing::RoutableEvent;
use crate::planes::PlaneType;
use std::any::Any;

/// Event type identifier for cross-crate events
pub type EventTypeId = &'static str;

/// All cross-crate events in the RVOIP system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RvoipCrossCrateEvent {
    /// Session-core to dialog-core events
    SessionToDialog(SessionToDialogEvent),

    /// Dialog-core to session-core events
    DialogToSession(DialogToSessionEvent),

    /// Session-core to media-core events
    SessionToMedia(SessionToMediaEvent),

    /// Media-core to session-core events
    MediaToSession(MediaToSessionEvent),

    /// Dialog-core to sip-transport events
    DialogToTransport(DialogToTransportEvent),

    /// Sip-transport to dialog-core events
    TransportToDialog(TransportToDialogEvent),

    /// Media-core to rtp-core events
    MediaToRtp(MediaToRtpEvent),

    /// Rtp-core to media-core events
    RtpToMedia(RtpToMediaEvent),
}

/// Trait for cross-crate events
pub trait CrossCrateEvent: Send + Sync + std::fmt::Debug {
    fn event_type(&self) -> EventTypeId;
    fn source_plane(&self) -> PlaneType;
    fn target_plane(&self) -> PlaneType;
    fn priority(&self) -> EventPriority;

    /// Convert to Any for downcasting (trait-based approach)
    fn as_any(&self) -> &dyn Any;
}

impl CrossCrateEvent for RvoipCrossCrateEvent {
    fn event_type(&self) -> EventTypeId {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => "session_to_dialog",
            RvoipCrossCrateEvent::DialogToSession(_) => "dialog_to_session",
            RvoipCrossCrateEvent::SessionToMedia(_) => "session_to_media",
            RvoipCrossCrateEvent::MediaToSession(_) => "media_to_session",
            RvoipCrossCrateEvent::DialogToTransport(_) => "dialog_to_transport",
            RvoipCrossCrateEvent::TransportToDialog(_) => "transport_to_dialog",
            RvoipCrossCrateEvent::MediaToRtp(_) => "media_to_rtp",
            RvoipCrossCrateEvent::RtpToMedia(_) => "rtp_to_media",
        }
    }

    fn source_plane(&self) -> PlaneType {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::DialogToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::SessionToMedia(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::MediaToSession(_) => PlaneType::Media,
            RvoipCrossCrateEvent::DialogToTransport(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::TransportToDialog(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::MediaToRtp(_) => PlaneType::Media,
            RvoipCrossCrateEvent::RtpToMedia(_) => PlaneType::Transport,
        }
    }

    fn target_plane(&self) -> PlaneType {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::DialogToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::SessionToMedia(_) => PlaneType::Media,
            RvoipCrossCrateEvent::MediaToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::DialogToTransport(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::TransportToDialog(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::MediaToRtp(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::RtpToMedia(_) => PlaneType::Media,
        }
    }

    fn priority(&self) -> EventPriority {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => EventPriority::High,
            RvoipCrossCrateEvent::DialogToSession(_) => EventPriority::High,
            RvoipCrossCrateEvent::SessionToMedia(_) => EventPriority::High,
            RvoipCrossCrateEvent::MediaToSession(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::DialogToTransport(_) => EventPriority::High,
            RvoipCrossCrateEvent::TransportToDialog(_) => EventPriority::High,
            RvoipCrossCrateEvent::MediaToRtp(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::RtpToMedia(_) => EventPriority::Normal,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Event for RvoipCrossCrateEvent {
    fn event_type() -> &'static str {
        "rvoip_cross_crate_event"
    }

    fn priority() -> EventPriority {
        EventPriority::High // Cross-crate events are high priority by default
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RoutableEvent for RvoipCrossCrateEvent {
    fn event_type(&self) -> &'static str {
        CrossCrateEvent::event_type(self)
    }

    fn session_id(&self) -> Option<&str> {
        // Extract session ID from the event if present
        match self {
            RvoipCrossCrateEvent::SessionToDialog(event) => match event {
                SessionToDialogEvent::InitiateCall { session_id, .. } => Some(session_id),
                SessionToDialogEvent::TerminateSession { session_id, .. } => Some(session_id),
                SessionToDialogEvent::HoldSession { session_id, .. } => Some(session_id),
                SessionToDialogEvent::ResumeSession { session_id, .. } => Some(session_id),
                SessionToDialogEvent::TransferCall { session_id, .. } => Some(session_id),
                SessionToDialogEvent::SendDtmf { session_id, .. } => Some(session_id),
                SessionToDialogEvent::StoreDialogMapping { session_id, .. } => Some(session_id),
                SessionToDialogEvent::ReferResponse { .. } => None, // No session_id in ReferResponse
                SessionToDialogEvent::SendRegisterResponse { .. } => None, // Transaction-based, no session_id
            },
            RvoipCrossCrateEvent::DialogToSession(event) => match event {
                DialogToSessionEvent::IncomingCall { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallStateChanged { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallEstablished { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallTerminated { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallCancelled { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SessionRefreshed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SessionRefreshFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::AuthRequired { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallRedirected { session_id, .. } => Some(session_id),
                DialogToSessionEvent::ReinviteGlare { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SessionIntervalTooSmall { session_id, .. } => {
                    Some(session_id)
                }
                DialogToSessionEvent::DtmfReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::DialogError { session_id, .. } => Some(session_id),
                DialogToSessionEvent::DialogCreated { .. } => None, // No session_id in DialogCreated
                DialogToSessionEvent::DialogStateChanged { session_id, .. } => Some(session_id),
                DialogToSessionEvent::ReinviteReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::TransferRequested { session_id, .. } => Some(session_id),
                DialogToSessionEvent::AckReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::RegistrationSuccess { session_id, .. } => Some(session_id),
                DialogToSessionEvent::RegistrationFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SubscriptionAccepted { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SubscriptionFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::NotifyReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::MessageDelivered { session_id, .. } => Some(session_id),
                DialogToSessionEvent::MessageFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::IncomingRegister { .. } => None, // No session_id yet for incoming REGISTER
                DialogToSessionEvent::OutboundFlowFailed { .. } => None, // Flow-level, not session-level
            },
            RvoipCrossCrateEvent::SessionToMedia(event) => match event {
                SessionToMediaEvent::StartMediaStream { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StopMediaStream { session_id, .. } => Some(session_id),
                SessionToMediaEvent::UpdateMediaStream { session_id, .. } => Some(session_id),
                SessionToMediaEvent::HoldMedia { session_id, .. } => Some(session_id),
                SessionToMediaEvent::ResumeMedia { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StartRecording { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StopRecording { session_id, .. } => Some(session_id),
                SessionToMediaEvent::PlayAudio { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StopAudio { session_id, .. } => Some(session_id),
            },
            RvoipCrossCrateEvent::MediaToSession(event) => match event {
                MediaToSessionEvent::MediaStreamStarted { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaStreamStopped { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaQualityUpdate { session_id, .. } => Some(session_id),
                MediaToSessionEvent::RecordingStarted { session_id, .. } => Some(session_id),
                MediaToSessionEvent::RecordingStopped { session_id, .. } => Some(session_id),
                MediaToSessionEvent::AudioPlaybackFinished { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaError { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaFlowEstablished { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaQualityDegraded { session_id, .. } => Some(session_id),
                MediaToSessionEvent::DtmfDetected { session_id, .. } => Some(session_id),
                MediaToSessionEvent::RtpTimeout { session_id, .. } => Some(session_id),
                MediaToSessionEvent::PacketLossThresholdExceeded { session_id, .. } => {
                    Some(session_id)
                }
            },
            RvoipCrossCrateEvent::DialogToTransport(_) => None, // Transport events don't have session context
            RvoipCrossCrateEvent::TransportToDialog(_) => None,
            RvoipCrossCrateEvent::MediaToRtp(event) => match event {
                MediaToRtpEvent::StartRtpStream { session_id, .. } => Some(session_id),
                MediaToRtpEvent::StopRtpStream { session_id, .. } => Some(session_id),
                MediaToRtpEvent::SendRtpPacket { session_id, .. } => Some(session_id),
                MediaToRtpEvent::UpdateRtpStream { session_id, .. } => Some(session_id),
            },
            RvoipCrossCrateEvent::RtpToMedia(event) => match event {
                RtpToMediaEvent::RtpStreamStarted { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpStreamStopped { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpPacketReceived { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpStatisticsUpdate { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpError { session_id, .. } => Some(session_id),
            },
        }
    }
}

// =============================================================================
// SESSION-CORE ↔ DIALOG-CORE EVENTS
// =============================================================================

/// Events sent from session-core to dialog-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SessionToDialogEvent {
    /// Request to initiate a new call
    InitiateCall {
        session_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
        headers: HashMap<String, String>,
    },

    /// Request to terminate a session
    TerminateSession { session_id: String, reason: String },

    /// Request to hold a session
    HoldSession { session_id: String },

    /// Request to resume a session from hold
    ResumeSession {
        session_id: String,
        sdp_offer: Option<String>,
    },

    /// Request to transfer a call
    TransferCall {
        session_id: String,
        target: String,
        transfer_type: TransferType,
    },

    /// Send DTMF tones
    SendDtmf { session_id: String, tones: String },

    /// Store dialog mapping (response to DialogCreated)
    StoreDialogMapping {
        session_id: String,
        dialog_id: String,
    },

    /// Response to REFER request (Accept/Reject decision)
    ReferResponse {
        transaction_id: String,
        accept: bool,
        status_code: u16,
        reason: String,
    },

    /// Send REGISTER response (401/200) - server-side
    SendRegisterResponse {
        transaction_id: String,
        status_code: u16,
        reason: String,
        www_authenticate: Option<String>, // For 401 challenge
        contact: Option<String>,          // For 200 OK
        expires: Option<u32>,             // For 200 OK
    },
}

/// Events sent from dialog-core to session-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DialogToSessionEvent {
    /// Incoming call notification
    IncomingCall {
        session_id: String,
        call_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
        headers: HashMap<String, String>,
        /// Transaction ID for sending responses
        transaction_id: String,
        /// Source address for responses
        source_addr: String,
    },

    /// Call state change notification
    CallStateChanged {
        session_id: String,
        new_state: CallState,
        reason: Option<String>,
    },

    /// Call successfully established
    CallEstablished {
        session_id: String,
        sdp_answer: Option<String>,
    },

    /// Call terminated notification
    CallTerminated {
        session_id: String,
        reason: TerminationReason,
    },

    /// Final failure response received for an outgoing request
    /// (3xx redirect, 4xx client error, 5xx server error, 6xx global failure).
    /// RFC 3261 §8.1.3 — the UAC transaction is complete on a final response.
    CallFailed {
        session_id: String,
        status_code: u16,
        reason_phrase: String,
    },

    /// Caller cancelled before the call was answered (RFC 3261 §15.1.2 —
    /// 487 Request Terminated after CANCEL). Distinct from CallFailed so
    /// applications can render "missed call" UX rather than "call failed".
    CallCancelled { session_id: String },

    /// RFC 4028 session-timer refresh succeeded (UPDATE or re-INVITE
    /// completed round-trip). Emitted once per successful refresh.
    SessionRefreshed {
        session_id: String,
        expires_secs: u32,
    },

    /// RFC 4028 session-timer refresh failed; the dialog has been torn
    /// down with BYE (§10). A subsequent CallTerminated will also fire.
    SessionRefreshFailed { session_id: String, reason: String },

    /// RFC 3261 §22.2 — server challenged the UAC request. Emitted on any
    /// 401 Unauthorized or 407 Proxy Authentication Required that carries a
    /// parseable challenge header. Method-agnostic: INVITE, REGISTER, and
    /// future auth-challenged requests all route through this variant. If
    /// the caller has credentials on file, session-core computes the
    /// digest response and retries; otherwise this converts to a final
    /// `CallFailed` / `RegistrationFailed` at the app level.
    AuthRequired {
        session_id: String,
        /// 401 or 407.
        status_code: u16,
        /// Raw challenge header value (e.g. `Digest realm="...", nonce="..."`).
        /// Passed verbatim to `auth-core::DigestAuthenticator::parse_challenge`.
        challenge: String,
        /// Pre-extracted realm, convenience for logging / app-level routing.
        /// Authoritative parse is still done by auth-core.
        realm: Option<String>,
    },

    /// 3xx redirect response received (RFC 3261 §8.1.3.4 / §21.3). The UAC
    /// SHOULD retry the INVITE against the first URI in `targets`. `q_values`
    /// carries the relative priority from Contact headers (RFC 3261 §20.10);
    /// each entry defaults to 1.0 when the server omits it.
    CallRedirected {
        session_id: String,
        status_code: u16,
        targets: Vec<String>,
        q_values: Vec<f32>,
    },

    /// 491 Request Pending for a mid-dialog request (RFC 3261 §14.1). The
    /// UAC SHOULD wait a random interval and retry. Emitted only for
    /// re-INVITEs (and UPDATEs) — call-setup INVITEs fall through the
    /// generic CallFailed path.
    ReinviteGlare { session_id: String },

    /// RFC 4028 §6 — 422 Session Interval Too Small on INVITE. The UAS
    /// requires a longer session interval than the UAC offered; its
    /// `Min-SE:` header (extracted into `min_se_secs`) carries the required
    /// floor. The UAC should resend the INVITE with a `Session-Expires`
    /// bumped to at least `min_se_secs`.
    ///
    /// session-core handles this transparently with a two-retry cap
    /// mirroring the 423 REGISTER-retry pattern. If the response is
    /// missing a parseable `Min-SE` header the event falls through to
    /// generic `CallFailed`.
    SessionIntervalTooSmall {
        session_id: String,
        /// Required minimum session interval, in seconds, parsed from the
        /// server's `Min-SE:` header.
        min_se_secs: u32,
    },

    /// DTMF tones received
    DtmfReceived { session_id: String, tones: String },

    /// Dialog error occurred
    DialogError {
        session_id: String,
        error: String,
        error_code: Option<u32>,
    },

    /// Dialog created (for session-core to track)
    DialogCreated { dialog_id: String, call_id: String },

    /// Dialog state changed
    DialogStateChanged {
        session_id: String,
        old_state: DialogState,
        new_state: DialogState,
    },

    /// Re-INVITE or UPDATE received (mid-dialog request). `method` is the
    /// uppercase SIP method string ("INVITE" or "UPDATE"); session-core
    /// uses it to dispatch to the correct state-table event.
    ReinviteReceived {
        session_id: String,
        sdp: Option<String>,
        method: String,
    },

    /// Transfer requested
    TransferRequested {
        session_id: String,
        refer_to: String,
        transfer_type: TransferType,
        transaction_id: String,
        /// Optional Referred-By header value from the REFER request.
        referred_by: Option<String>,
        /// Optional Replaces value, either from a Replaces header or the
        /// Refer-To URI for attended-transfer primitives.
        replaces: Option<String>,
    },

    /// ACK received (for UAS state transitions)
    AckReceived {
        session_id: String,
        sdp: Option<String>,
    },

    /// Registration successful
    RegistrationSuccess { session_id: String },

    /// Registration failed
    RegistrationFailed {
        session_id: String,
        status_code: u16,
    },

    /// Subscription accepted
    SubscriptionAccepted { session_id: String },

    /// Subscription failed
    SubscriptionFailed {
        session_id: String,
        status_code: u16,
    },

    /// NOTIFY received.
    ///
    /// Published by dialog-core after validating an inbound NOTIFY and
    /// sending the 200 OK (RFC 6665). Session-core uses this to surface
    /// `Event::NotifyReceived` on the public event stream; for REFER
    /// subscriptions (`event_package == "refer"`) with a
    /// `message/sipfrag` body it also parses the sipfrag status line
    /// into `Event::TransferProgress` / `TransferCompleted` / `TransferFailed`
    /// so the transferor (including b2bua wrappers) can observe the
    /// transferee's progress.
    NotifyReceived {
        session_id: String,
        event_package: String,
        /// Raw `Subscription-State:` header value (unparsed), e.g.
        /// `"active;expires=3600"` or `"terminated;reason=noresource"`.
        subscription_state: Option<String>,
        /// Raw `Content-Type:` header value, e.g. `"message/sipfrag"`.
        content_type: Option<String>,
        body: Option<String>,
    },

    /// MESSAGE delivered
    MessageDelivered { session_id: String },

    /// MESSAGE delivery failed
    MessageFailed {
        session_id: String,
        status_code: u16,
    },

    /// Incoming REGISTER request (server-side)
    IncomingRegister {
        transaction_id: String,
        from_uri: String,
        to_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>, // Authorization header if present
        call_id: String,
    },

    /// RFC 5626 outbound flow has failed — the keep-alive ping either
    /// timed out, saw a transport-level connection close, or hit an
    /// unrecoverable send error. Session-core debounces this event per
    /// AoR and triggers a fresh REGISTER to re-establish the flow
    /// without waiting for registration expiry (§4.4.1 flow recovery).
    ///
    /// Flow-level, not session-level: `session_id()` returns `None`.
    OutboundFlowFailed {
        /// AoR (To URI of the REGISTER that established the flow,
        /// normalized to string form).
        aor: String,
        /// RFC 5626 §4.2 `reg-id` of the failed flow.
        reg_id: u32,
        /// RFC 5626 §4.1 instance URN of the UA.
        instance: String,
        /// Human-readable failure cause (`"PongTimeout"`,
        /// `"ConnectionClosed"`, or `"SendError"`) — used for
        /// telemetry and log correlation.
        reason: String,
    },
}

// =============================================================================
// SESSION-CORE ↔ MEDIA-CORE EVENTS
// =============================================================================

/// Events sent from session-core to media-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SessionToMediaEvent {
    /// Start media stream for session
    StartMediaStream {
        session_id: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
        media_config: MediaStreamConfig,
    },

    /// Stop media stream for session
    StopMediaStream { session_id: String },

    /// Update media stream configuration
    UpdateMediaStream {
        session_id: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
    },

    /// Hold media stream
    HoldMedia { session_id: String },

    /// Resume media stream
    ResumeMedia { session_id: String },

    /// Start recording
    StartRecording {
        session_id: String,
        file_path: String,
        format: RecordingFormat,
    },

    /// Stop recording
    StopRecording { session_id: String },

    /// Play audio file
    PlayAudio {
        session_id: String,
        file_path: String,
        loop_count: Option<u32>,
    },

    /// Stop audio playback
    StopAudio { session_id: String },
}

/// Events sent from media-core to session-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MediaToSessionEvent {
    /// Media stream started successfully
    MediaStreamStarted {
        session_id: String,
        local_port: u16,
        codec: String,
    },

    /// Media stream stopped
    MediaStreamStopped { session_id: String, reason: String },

    /// Media quality update
    MediaQualityUpdate {
        session_id: String,
        quality_metrics: MediaQualityMetrics,
    },

    /// Recording started
    RecordingStarted {
        session_id: String,
        file_path: String,
    },

    /// Recording stopped
    RecordingStopped {
        session_id: String,
        file_path: String,
        duration_ms: u64,
    },

    /// Audio playback finished
    AudioPlaybackFinished { session_id: String },

    /// Media error occurred
    MediaError {
        session_id: String,
        error: String,
        error_code: Option<u32>,
    },

    /// Media flow established
    MediaFlowEstablished { session_id: String },

    /// Media quality degraded
    MediaQualityDegraded {
        session_id: String,
        metrics: MediaQualityMetrics,
        severity: QualitySeverity,
    },

    /// DTMF detected
    DtmfDetected {
        session_id: String,
        digit: char,
        duration_ms: u32,
    },

    /// RTP timeout
    RtpTimeout {
        session_id: String,
        last_packet_time: u64,
    },

    /// Packet loss threshold exceeded
    PacketLossThresholdExceeded {
        session_id: String,
        loss_percentage: f32,
    },
}

// =============================================================================
// DIALOG-CORE ↔ SIP-TRANSPORT EVENTS
// =============================================================================

/// Events sent from dialog-core to sip-transport
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DialogToTransportEvent {
    /// Send SIP message
    SendSipMessage {
        destination: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<String>,
        transaction_id: Option<String>,
    },

    /// Send SIP response
    SendSipResponse {
        transaction_id: String,
        status_code: u16,
        reason_phrase: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    },

    /// Register SIP endpoint
    RegisterEndpoint {
        uri: String,
        expires: Option<u32>,
        contact: Option<String>,
    },

    /// Unregister SIP endpoint
    UnregisterEndpoint { uri: String },
}

/// Events sent from sip-transport to dialog-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransportToDialogEvent {
    /// SIP message received
    SipMessageReceived {
        source: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<String>,
        transaction_id: String,
    },

    /// SIP response received
    SipResponseReceived {
        transaction_id: String,
        status_code: u16,
        reason_phrase: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    },

    /// Transport error occurred
    TransportError {
        error: String,
        transaction_id: Option<String>,
    },

    /// Registration status update
    RegistrationStatusUpdate {
        uri: String,
        status: RegistrationStatus,
        expires: Option<u32>,
    },
}

// =============================================================================
// MEDIA-CORE ↔ RTP-CORE EVENTS
// =============================================================================

/// Events sent from media-core to rtp-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MediaToRtpEvent {
    /// Start RTP stream
    StartRtpStream {
        session_id: String,
        local_port: u16,
        remote_address: String,
        remote_port: u16,
        payload_type: u8,
        codec: String,
    },

    /// Stop RTP stream
    StopRtpStream { session_id: String },

    /// Send RTP packet
    SendRtpPacket {
        session_id: String,
        payload: Vec<u8>,
        timestamp: u32,
        sequence_number: u16,
    },

    /// Update RTP stream parameters
    UpdateRtpStream {
        session_id: String,
        remote_address: Option<String>,
        remote_port: Option<u16>,
    },
}

/// Events sent from rtp-core to media-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RtpToMediaEvent {
    /// RTP stream started
    RtpStreamStarted { session_id: String, local_port: u16 },

    /// RTP stream stopped
    RtpStreamStopped { session_id: String, reason: String },

    /// RTP packet received
    RtpPacketReceived {
        session_id: String,
        payload: Vec<u8>,
        timestamp: u32,
        sequence_number: u16,
        payload_type: u8,
    },

    /// RTP statistics update
    RtpStatisticsUpdate {
        session_id: String,
        stats: RtpStatistics,
    },

    /// RTP error occurred
    RtpError { session_id: String, error: String },
}

// =============================================================================
// SUPPORTING TYPES
// =============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CallState {
    Initiating,
    Ringing,
    Active,
    OnHold,
    Transferring,
    Terminating,
    Terminated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TerminationReason {
    LocalHangup,
    RemoteHangup,
    Rejected(String),
    Error(String),
    Timeout,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DialogState {
    Initial,
    Early,
    Confirmed,
    Recovering,
    Terminated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum QualitySeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransferType {
    Blind,
    Attended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaStreamConfig {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub enable_dtx: bool,
    pub enable_fec: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecordingFormat {
    Wav,
    Mp3,
    Flac,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaQualityMetrics {
    pub mos_score: f64,
    pub packet_loss: f64,
    pub jitter_ms: f64,
    pub delay_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RegistrationStatus {
    Registered,
    Unregistered,
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RtpStatistics {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packet_loss_rate: f64,
    pub jitter_ms: f64,
}

/// Helper functions for creating cross-crate events
impl RvoipCrossCrateEvent {
    /// Create a session to dialog initiate call event
    pub fn initiate_call(
        session_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::SessionToDialog(SessionToDialogEvent::InitiateCall {
            session_id,
            from,
            to,
            sdp_offer,
            headers: HashMap::new(),
        })
    }

    /// Create an incoming call event
    pub fn incoming_call(
        session_id: String,
        call_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::IncomingCall {
            session_id,
            call_id,
            from,
            to,
            sdp_offer,
            headers: HashMap::new(),
            transaction_id: String::new(), // Must be set by caller
            source_addr: String::new(),    // Must be set by caller
        })
    }

    /// Create a call state changed event
    pub fn call_state_changed(
        session_id: String,
        new_state: CallState,
        reason: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallStateChanged {
            session_id,
            new_state,
            reason,
        })
    }

    /// Create a start media stream event
    pub fn start_media_stream(
        session_id: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
        config: MediaStreamConfig,
    ) -> Self {
        RvoipCrossCrateEvent::SessionToMedia(SessionToMediaEvent::StartMediaStream {
            session_id,
            local_sdp,
            remote_sdp,
            media_config: config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_identification() {
        let event = RvoipCrossCrateEvent::initiate_call(
            "test_session".to_string(),
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
            None,
        );

        assert_eq!(CrossCrateEvent::event_type(&event), "session_to_dialog");
        assert_eq!(event.source_plane(), PlaneType::Signaling);
        assert_eq!(event.target_plane(), PlaneType::Signaling);
        assert_eq!(event.priority(), EventPriority::High);
    }

    #[test]
    fn test_event_serialization() {
        let event = RvoipCrossCrateEvent::call_state_changed(
            "test_session".to_string(),
            CallState::Active,
            None,
        );

        // Test that events can be serialized and deserialized
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: RvoipCrossCrateEvent = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            CrossCrateEvent::event_type(&deserialized),
            CrossCrateEvent::event_type(&event)
        );
    }
}
